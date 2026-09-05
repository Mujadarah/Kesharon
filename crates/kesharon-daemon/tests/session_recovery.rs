use std::sync::{Arc, Mutex};

use kesharon_application::{
    ApplicationError, IdGenerator, RepositoryInspection, RepositoryService, StateRepository,
};
use kesharon_daemon::{DaemonSession, SharedStateRepository, SqliteStateRepository};
use kesharon_domain::{Project, ProjectId, ResourceBudget, Task, TaskCheckpoint, TaskId};
use kesharon_protocol::{ClientRequest, ErrorCode, RequestMethod, ResponsePayload};

struct StubRepository;

impl RepositoryService for StubRepository {
    fn inspect(
        &self,
        requested_path: &str,
    ) -> Result<RepositoryInspection, kesharon_application::ApplicationError> {
        Ok(RepositoryInspection {
            canonical_root: requested_path.to_owned(),
            display_name: "test-recovered-project".to_owned(),
        })
    }
}

struct FixedId;

impl IdGenerator for FixedId {
    fn next_id(&self) -> String {
        "project-fixed-1".to_owned()
    }
}

struct FailingSaveRepository;

impl StateRepository for FailingSaveRepository {
    fn save_project(&mut self, _project: &Project) -> Result<(), ApplicationError> {
        Err(ApplicationError::Storage("simulated disk full".into()))
    }
    fn load_project(&self, _id: &str) -> Result<Option<Project>, ApplicationError> {
        Ok(None)
    }
    fn load_last_active_project(&self) -> Result<Option<Project>, ApplicationError> {
        Ok(None)
    }
    fn save_task(&mut self, _task: &Task) -> Result<(), ApplicationError> {
        Ok(())
    }
    fn load_task(&self, _id: &str) -> Result<Option<Task>, ApplicationError> {
        Ok(None)
    }
    fn load_active_task(&self, _project_id: &str) -> Result<Option<Task>, ApplicationError> {
        Ok(None)
    }
    fn save_checkpoint(
        &mut self,
        _task_id: &str,
        _checkpoint: &TaskCheckpoint,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
    fn list_checkpoints(&self, _task_id: &str) -> Result<Vec<TaskCheckpoint>, ApplicationError> {
        Ok(vec![])
    }
    fn record_idempotency(
        &mut self,
        _key: &str,
        _payload_hash: &str,
        _response_json: &str,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
    fn find_idempotency(&self, _key: &str) -> Result<Option<String>, ApplicationError> {
        Ok(None)
    }
}

#[test]
fn daemon_session_recovers_last_active_project_and_task_on_startup() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory db");
    let project = Project::new(
        ProjectId::new("proj-recovered-42").expect("valid id"),
        "My Persisted Project",
        "D:\\repos\\persisted",
        true,
    )
    .expect("valid project");
    storage.save_project(&project).expect("saved project");

    let budget =
        ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1).expect("valid budget");
    let task = Task::new(
        TaskId::new("task-active-99").expect("valid id"),
        "Active task to recover",
        budget,
    )
    .expect("valid task");
    storage.save_task(&task).expect("saved active task");

    let storage_boxed: Box<dyn StateRepository + Send + Sync> = Box::new(storage);
    let storage_arc: SharedStateRepository = Arc::new(Mutex::new(storage_boxed));

    let session = DaemonSession::with_storage(
        "stream-recovery-1",
        Arc::new(StubRepository),
        Arc::new(FixedId),
        Some(storage_arc),
    )
    .expect("session created with storage");

    let subscription = session.subscribe().expect("subscription succeeds");
    let recovered_project = subscription
        .snapshot
        .project()
        .expect("project is recovered");
    assert_eq!(recovered_project.id, "proj-recovered-42");
    assert_eq!(recovered_project.display_name, "My Persisted Project");
    assert_eq!(recovered_project.canonical_root, "D:\\repos\\persisted");
    assert!(recovered_project.trusted);

    let recovered_task = session
        .active_task()
        .expect("active_task query succeeds")
        .expect("active task is recovered");
    assert_eq!(recovered_task.id().as_str(), "task-active-99");
    assert_eq!(recovered_task.goal(), "Active task to recover");
}

#[test]
fn daemon_session_persists_opened_project_to_storage_and_recovers_in_subsequent_session() {
    let temp_dir =
        std::env::temp_dir().join(format!("kesharon-daemon-recovery-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&temp_dir).expect("created temp dir");
    let db_path = temp_dir.join("state.db");

    {
        let storage = SqliteStateRepository::open(&db_path).expect("opened db");
        let storage_boxed: Box<dyn StateRepository + Send + Sync> = Box::new(storage);
        let storage_arc: SharedStateRepository = Arc::new(Mutex::new(storage_boxed));
        let session = DaemonSession::with_storage(
            "stream-session-1",
            Arc::new(StubRepository),
            Arc::new(FixedId),
            Some(storage_arc),
        )
        .expect("session 1 created");

        let open_req = ClientRequest::new(
            "req-open-1",
            RequestMethod::OpenProject {
                path: "D:\\repos\\new-project".into(),
            },
            Some("idem-key-1".into()),
        )
        .expect("valid request");

        let response = session.dispatch(&open_req);
        assert!(matches!(
            response.result(),
            Some(ResponsePayload::ProjectOpened { .. })
        ));
    }

    // Now open a brand new DaemonSession with the same database file
    {
        let storage = SqliteStateRepository::open(&db_path).expect("re-opened db");
        let storage_boxed: Box<dyn StateRepository + Send + Sync> = Box::new(storage);
        let storage_arc: SharedStateRepository = Arc::new(Mutex::new(storage_boxed));
        let session = DaemonSession::with_storage(
            "stream-session-2",
            Arc::new(StubRepository),
            Arc::new(FixedId),
            Some(storage_arc),
        )
        .expect("session 2 created");

        let subscription = session.subscribe().expect("subscription succeeds");
        let project = subscription.snapshot.project().expect("project recovered");
        assert_eq!(project.id, "project-fixed-1");
        assert_eq!(project.display_name, "test-recovered-project");
        assert_eq!(project.canonical_root, "D:\\repos\\new-project");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn daemon_session_fails_open_project_when_storage_save_fails() {
    let failing_storage: Box<dyn StateRepository + Send + Sync> = Box::new(FailingSaveRepository);
    let storage_arc: SharedStateRepository = Arc::new(Mutex::new(failing_storage));

    let session = DaemonSession::with_storage(
        "stream-fail-1",
        Arc::new(StubRepository),
        Arc::new(FixedId),
        Some(storage_arc),
    )
    .expect("session created");

    let open_req = ClientRequest::new(
        "req-fail-open",
        RequestMethod::OpenProject {
            path: "D:\\repos\\failing-project".into(),
        },
        Some("idem-fail-key".into()),
    )
    .expect("valid request");

    let response = session.dispatch(&open_req);
    let err = response.error().expect("expected error response");
    assert_eq!(err.code(), ErrorCode::InternalError);
    assert_eq!(err.message(), "Storage error encountered");

    // Retrying with the same idempotency key must replay the cached failure response
    let retry_req = ClientRequest::new(
        "req-fail-open-retry",
        RequestMethod::OpenProject {
            path: "D:\\repos\\failing-project".into(),
        },
        Some("idem-fail-key".into()),
    )
    .expect("valid request");

    let retry_response = session.dispatch(&retry_req);
    let retry_err = retry_response
        .error()
        .expect("expected cached error response");
    assert_eq!(retry_err.code(), ErrorCode::InternalError);
    assert_eq!(retry_err.message(), "Storage error encountered");
    assert_eq!(retry_response.request_id(), "req-fail-open-retry");
}
