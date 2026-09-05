use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use kesharon_application::{
    ApplicationError, CancellationSignal, IdGenerator, OpenProject, OpenProjectCommand,
    RepositoryService, SessionRecovery, StateRepository,
};
use kesharon_protocol::{
    CancellationOutcome, ClientRequest, DaemonEvent, DaemonEventPayload, ErrorCode, HealthStatus,
    OperationKind, OperationSnapshot, OperationStatus, PROTOCOL_VERSION, ProjectSnapshot,
    RequestMethod, ResponsePayload, ServerResponse, StreamMessage, WorkspaceSnapshot,
};

use crate::repository::{UuidIds, production_repository};
use crate::storage::SqliteStateRepository;

const IDEMPOTENCY_LIMIT: usize = 256;
const SUBSCRIBER_CAPACITY: usize = 64;

struct CancellationFlag(AtomicBool);

impl CancellationFlag {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl CancellationSignal for CancellationFlag {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

struct ActiveOperation {
    cancellation: Arc<CancellationFlag>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MutationFingerprint {
    OpenProject { path: String },
    CancelRequest { target_request_id: String },
}

struct CompletedMutation {
    fingerprint: MutationFingerprint,
    response: ServerResponse,
}

struct SessionState {
    project: Option<ProjectSnapshot>,
    active: HashMap<String, ActiveOperation>,
    in_flight: HashMap<String, MutationFingerprint>,
    completed: HashMap<String, CompletedMutation>,
    completed_order: VecDeque<String>,
    completed_requests: HashSet<String>,
    completed_request_order: VecDeque<String>,
    sequence: u64,
    subscriber: Option<SyncSender<StreamMessage>>,
}

impl SessionState {
    fn new() -> Self {
        Self {
            project: None,
            active: HashMap::new(),
            in_flight: HashMap::new(),
            completed: HashMap::new(),
            completed_order: VecDeque::new(),
            completed_requests: HashSet::new(),
            completed_request_order: VecDeque::new(),
            sequence: 0,
            subscriber: None,
        }
    }
}

pub struct SessionSubscription {
    pub snapshot: WorkspaceSnapshot,
    pub receiver: Receiver<StreamMessage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionError {
    StateUnavailable,
    SequenceExhausted,
}

pub type SharedStateRepository = Arc<Mutex<Box<dyn StateRepository + Send + Sync>>>;

pub struct DaemonSession {
    stream_id: String,
    repository: Arc<dyn RepositoryService + Send + Sync>,
    ids: Arc<dyn IdGenerator + Send + Sync>,
    state_repository: Option<SharedStateRepository>,
    state: Mutex<SessionState>,
}

impl DaemonSession {
    pub fn new(
        stream_id: impl Into<String>,
        repository: Arc<dyn RepositoryService + Send + Sync>,
        ids: Arc<dyn IdGenerator + Send + Sync>,
    ) -> Self {
        Self::with_storage(stream_id, repository, ids, None)
    }

    pub fn with_storage(
        stream_id: impl Into<String>,
        repository: Arc<dyn RepositoryService + Send + Sync>,
        ids: Arc<dyn IdGenerator + Send + Sync>,
        state_repository: Option<SharedStateRepository>,
    ) -> Self {
        let initial_project = state_repository.as_ref().and_then(|repo_mutex| {
            repo_mutex.lock().ok().and_then(|repo| {
                SessionRecovery::new(repo.as_ref())
                    .execute()
                    .ok()
                    .and_then(|recovered| {
                        recovered.project().map(|project| ProjectSnapshot {
                            id: project.id().as_str().to_owned(),
                            display_name: project.display_name().to_owned(),
                            canonical_root: project.canonical_root().to_owned(),
                            trusted: project.is_trusted(),
                        })
                    })
            })
        });

        let mut initial_state = SessionState::new();
        initial_state.project = initial_project;

        Self {
            stream_id: stream_id.into(),
            repository,
            ids,
            state_repository,
            state: Mutex::new(initial_state),
        }
    }

    pub(crate) fn production() -> Self {
        let storage: Option<SharedStateRepository> =
            if let Some(path) = std::env::var_os("KESHARON_STATE_DB") {
                SqliteStateRepository::open(path)
                    .map(|repo| {
                        Arc::new(Mutex::new(
                            Box::new(repo) as Box<dyn StateRepository + Send + Sync>
                        ))
                    })
                    .ok()
            } else {
                SqliteStateRepository::in_memory()
                    .map(|repo| {
                        Arc::new(Mutex::new(
                            Box::new(repo) as Box<dyn StateRepository + Send + Sync>
                        ))
                    })
                    .ok()
            };
        Self::with_storage(
            uuid::Uuid::now_v7().to_string(),
            production_repository(),
            Arc::new(UuidIds),
            storage,
        )
    }

    pub fn stream_id(&self) -> String {
        self.stream_id.clone()
    }

    pub fn subscribe(&self) -> Result<SessionSubscription, SessionError> {
        let (sender, receiver) = sync_channel(SUBSCRIBER_CAPACITY);
        let mut state = self
            .state
            .lock()
            .map_err(|_| SessionError::StateUnavailable)?;
        let snapshot = self.snapshot_locked(&state);
        state.subscriber = Some(sender);
        Ok(SessionSubscription { snapshot, receiver })
    }

    pub fn dispatch(&self, request: &ClientRequest) -> ServerResponse {
        match request.method() {
            RequestMethod::Health => ServerResponse::success(
                request.request_id(),
                ResponsePayload::Health {
                    status: HealthStatus::Ready,
                    protocol_version: PROTOCOL_VERSION,
                },
            ),
            RequestMethod::OpenProject { path } => self.open_project(request, path),
            RequestMethod::CancelRequest { target_request_id } => {
                self.cancel(request, target_request_id)
            }
            RequestMethod::SubscribeEvents => ServerResponse::success(
                request.request_id(),
                ResponsePayload::SubscriptionReady {
                    stream_id: self.stream_id.clone(),
                },
            ),
        }
    }

    fn open_project(&self, request: &ClientRequest, path: &str) -> ServerResponse {
        let Some(key) = request.idempotency_key() else {
            return internal_error(request.request_id());
        };
        let fingerprint = MutationFingerprint::OpenProject {
            path: path.to_owned(),
        };
        let cancellation = {
            let Ok(mut state) = self.state.lock() else {
                return internal_error(request.request_id());
            };
            if let Some(completed) = state.completed.get(key) {
                if completed.fingerprint != fingerprint {
                    return idempotency_conflict(request.request_id());
                }
                return completed.response.with_request_id(request.request_id());
            }
            if let Some(in_flight) = state.in_flight.get(key) {
                if in_flight != &fingerprint {
                    return idempotency_conflict(request.request_id());
                }
                return ServerResponse::failure(
                    request.request_id(),
                    ErrorCode::RequestInProgress,
                    "A request with this idempotency key is still running",
                );
            }
            if state.active.contains_key(request.request_id()) {
                return ServerResponse::failure(
                    request.request_id(),
                    ErrorCode::RequestInProgress,
                    "A request with this request ID is still running",
                );
            }
            let cancellation = Arc::new(CancellationFlag::new());
            state.in_flight.insert(key.to_owned(), fingerprint.clone());
            state.active.insert(
                request.request_id().to_owned(),
                ActiveOperation {
                    cancellation: Arc::clone(&cancellation),
                },
            );
            if self
                .emit_locked(
                    &mut state,
                    DaemonEventPayload::OperationStarted {
                        request_id: request.request_id().to_owned(),
                        kind: OperationKind::OpenProject,
                    },
                )
                .is_err()
            {
                state.active.remove(request.request_id());
                state.in_flight.remove(key);
                return internal_error(request.request_id());
            }
            cancellation
        };

        let command = OpenProjectCommand {
            requested_path: path.to_owned(),
            trusted: false,
        };
        let result = OpenProject::new(self.repository.as_ref(), self.ids.as_ref())
            .execute(&command, cancellation.as_ref());
        let Ok(mut state) = self.state.lock() else {
            return internal_error(request.request_id());
        };
        state.active.remove(request.request_id());
        state.in_flight.remove(key);
        let response = match result {
            Ok(project) => {
                if let Some(repo_mutex) = &self.state_repository
                    && let Ok(mut repo) = repo_mutex.lock()
                {
                    let _ = repo.save_project(&project);
                }
                self.complete_success(request, &mut state, &project)
            }
            Err(ApplicationError::Cancelled) => self.complete_cancellation(request, &mut state),
            Err(error) => self.complete_failure(request, &mut state, &error),
        };
        remember_completed(
            &mut state,
            key,
            fingerprint,
            request.request_id(),
            response.clone(),
        );
        response
    }

    fn complete_success(
        &self,
        request: &ClientRequest,
        state: &mut SessionState,
        project: &kesharon_domain::Project,
    ) -> ServerResponse {
        let snapshot = ProjectSnapshot {
            id: project.id().as_str().to_owned(),
            display_name: project.display_name().to_owned(),
            canonical_root: project.canonical_root().to_owned(),
            trusted: project.is_trusted(),
        };
        if self
            .emit_locked(
                state,
                DaemonEventPayload::ProjectOpened {
                    request_id: request.request_id().to_owned(),
                    project: snapshot.clone(),
                },
            )
            .is_err()
        {
            return internal_error(request.request_id());
        }
        state.project = Some(snapshot.clone());
        ServerResponse::success(
            request.request_id(),
            ResponsePayload::ProjectOpened { project: snapshot },
        )
    }

    fn complete_cancellation(
        &self,
        request: &ClientRequest,
        state: &mut SessionState,
    ) -> ServerResponse {
        if self
            .emit_locked(
                state,
                DaemonEventPayload::OperationCancelled {
                    request_id: request.request_id().to_owned(),
                },
            )
            .is_err()
        {
            return internal_error(request.request_id());
        }
        ServerResponse::failure(
            request.request_id(),
            ErrorCode::OperationCancelled,
            "Project opening was cancelled",
        )
    }

    fn complete_failure(
        &self,
        request: &ClientRequest,
        state: &mut SessionState,
        error: &ApplicationError,
    ) -> ServerResponse {
        let (code, message) = classify_application_error(error);
        if self
            .emit_locked(
                state,
                DaemonEventPayload::OperationFailed {
                    request_id: request.request_id().to_owned(),
                    code,
                    message: message.to_owned(),
                },
            )
            .is_err()
        {
            return internal_error(request.request_id());
        }
        ServerResponse::failure(request.request_id(), code, message)
    }

    fn cancel(&self, request: &ClientRequest, target_request_id: &str) -> ServerResponse {
        let Some(key) = request.idempotency_key() else {
            return internal_error(request.request_id());
        };
        let fingerprint = MutationFingerprint::CancelRequest {
            target_request_id: target_request_id.to_owned(),
        };
        let Ok(mut state) = self.state.lock() else {
            return internal_error(request.request_id());
        };
        if let Some(completed) = state.completed.get(key) {
            if completed.fingerprint != fingerprint {
                return idempotency_conflict(request.request_id());
            }
            return completed.response.with_request_id(request.request_id());
        }
        if let Some(in_flight) = state.in_flight.get(key) {
            if in_flight != &fingerprint {
                return idempotency_conflict(request.request_id());
            }
            return ServerResponse::failure(
                request.request_id(),
                ErrorCode::RequestInProgress,
                "A request with this idempotency key is still running",
            );
        }
        let outcome = if let Some(operation) = state.active.get(target_request_id) {
            operation.cancellation.cancel();
            CancellationOutcome::Accepted
        } else if state.completed_requests.contains(target_request_id) {
            CancellationOutcome::AlreadyFinished
        } else {
            CancellationOutcome::NotFound
        };
        let response = ServerResponse::success(
            request.request_id(),
            ResponsePayload::Cancellation {
                target_request_id: target_request_id.to_owned(),
                outcome,
            },
        );
        remember_completed(
            &mut state,
            key,
            fingerprint,
            request.request_id(),
            response.clone(),
        );
        response
    }

    fn snapshot_locked(&self, state: &SessionState) -> WorkspaceSnapshot {
        let active_operations = state
            .active
            .keys()
            .map(|request_id| OperationSnapshot {
                request_id: request_id.clone(),
                kind: OperationKind::OpenProject,
                status: OperationStatus::Running,
            })
            .collect();
        WorkspaceSnapshot::new(
            self.stream_id.clone(),
            state.sequence,
            state.project.clone(),
            active_operations,
        )
    }

    fn emit_locked(
        &self,
        state: &mut SessionState,
        payload: DaemonEventPayload,
    ) -> Result<(), SessionError> {
        state.sequence = state
            .sequence
            .checked_add(1)
            .ok_or(SessionError::SequenceExhausted)?;
        let event = DaemonEvent::new(self.stream_id.clone(), state.sequence, payload)
            .map_err(|_| SessionError::SequenceExhausted)?;
        if state.subscriber.as_ref().is_some_and(|subscriber| {
            matches!(
                subscriber.try_send(StreamMessage::Event { event }),
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_))
            )
        }) {
            state.subscriber = None;
        }
        Ok(())
    }
}

fn remember_completed(
    state: &mut SessionState,
    key: &str,
    fingerprint: MutationFingerprint,
    request_id: &str,
    response: ServerResponse,
) {
    if !state.completed.contains_key(key) {
        state.completed_order.push_back(key.to_owned());
    }
    state.completed.insert(
        key.to_owned(),
        CompletedMutation {
            fingerprint,
            response,
        },
    );
    if state.completed_requests.insert(request_id.to_owned()) {
        state
            .completed_request_order
            .push_back(request_id.to_owned());
    }
    while state.completed_order.len() > IDEMPOTENCY_LIMIT {
        if let Some(expired) = state.completed_order.pop_front() {
            state.completed.remove(&expired);
        }
    }
    while state.completed_request_order.len() > IDEMPOTENCY_LIMIT {
        if let Some(expired) = state.completed_request_order.pop_front() {
            state.completed_requests.remove(&expired);
        }
    }
}

fn classify_application_error(error: &ApplicationError) -> (ErrorCode, &'static str) {
    match error {
        ApplicationError::Repository(message) if message == "notGitRepository" => (
            ErrorCode::NotGitRepository,
            "Selected directory is not a Git worktree",
        ),
        ApplicationError::Repository(_) => (
            ErrorCode::ProjectPathUnavailable,
            "Selected directory could not be opened",
        ),
        ApplicationError::InvalidProject(_) => {
            (ErrorCode::InvalidRequest, "Project identity is invalid")
        }
        ApplicationError::Storage(_) => (ErrorCode::InternalError, "Storage error encountered"),
        ApplicationError::Vault(_) => (
            ErrorCode::InternalError,
            "Credential vault error encountered",
        ),
        ApplicationError::Cancelled => (
            ErrorCode::OperationCancelled,
            "Project opening was cancelled",
        ),
    }
}

fn idempotency_conflict(request_id: &str) -> ServerResponse {
    ServerResponse::failure(
        request_id,
        ErrorCode::InvalidRequest,
        "Idempotency key was already used for a different mutation",
    )
}

fn internal_error(request_id: &str) -> ServerResponse {
    ServerResponse::failure(
        request_id,
        ErrorCode::InternalError,
        "Daemon session state is unavailable",
    )
}
