use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use kesharon_application::{
    ApplicationError, IdGenerator, RepositoryInspection, RepositoryService,
};
use kesharon_daemon::DaemonSession;
use kesharon_protocol::{
    CancellationOutcome, ClientRequest, DaemonEventPayload, ErrorCode, RequestMethod,
    ResponsePayload, StreamMessage,
};

struct FixedRepository;

impl RepositoryService for FixedRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        Ok(RepositoryInspection {
            canonical_root: requested_path.into(),
            display_name: "Kesharon".into(),
        })
    }
}

struct SequenceIds(AtomicUsize);

impl SequenceIds {
    const fn new() -> Self {
        Self(AtomicUsize::new(1))
    }
}

impl IdGenerator for SequenceIds {
    fn next_id(&self) -> String {
        format!("project-{}", self.0.fetch_add(1, Ordering::Relaxed))
    }
}

fn session(repository: Arc<dyn RepositoryService + Send + Sync>) -> Arc<DaemonSession> {
    Arc::new(DaemonSession::new(
        "stream-1",
        repository,
        Arc::new(SequenceIds::new()),
    ))
}

fn open_request(request_id: &str, key: &str) -> ClientRequest {
    open_request_for(request_id, key, r"D:\code\kesharon")
}

fn open_request_for(request_id: &str, key: &str, path: &str) -> ClientRequest {
    ClientRequest::new(
        request_id,
        RequestMethod::OpenProject { path: path.into() },
        Some(key.into()),
    )
    .expect("open request is valid")
}

#[test]
fn open_project_updates_snapshot_and_emits_monotonic_terminal_events() {
    let session = session(Arc::new(FixedRepository));
    let subscription = session.subscribe().expect("subscription is available");

    let response = session.dispatch(&open_request("request-open-1", "open-key-1"));

    assert!(matches!(
        response.result(),
        Some(ResponsePayload::ProjectOpened { project })
            if project.id == "project-1" && !project.trusted
    ));
    let started = subscription
        .receiver
        .recv()
        .expect("started event is delivered");
    let opened = subscription
        .receiver
        .recv()
        .expect("terminal event is delivered");
    assert!(matches!(
        started,
        StreamMessage::Event { event }
            if event.sequence() == 1
                && matches!(event.payload(), DaemonEventPayload::OperationStarted { .. })
    ));
    assert!(matches!(
        opened,
        StreamMessage::Event { event }
            if event.sequence() == 2
                && matches!(event.payload(), DaemonEventPayload::ProjectOpened { .. })
    ));

    let snapshot = session
        .subscribe()
        .expect("replacement subscription is available")
        .snapshot;
    assert_eq!(snapshot.last_sequence(), 2);
    assert_eq!(
        snapshot
            .project()
            .map(|project| project.display_name.as_str()),
        Some("Kesharon")
    );
    assert!(snapshot.active_operations().is_empty());
}

#[test]
fn duplicate_idempotency_key_replays_result_without_duplicate_events() {
    let session = session(Arc::new(FixedRepository));
    let subscription = session.subscribe().expect("subscription is available");
    let request = open_request("request-open-1", "open-key-1");

    let first = session.dispatch(&request);
    let second = session.dispatch(&open_request("request-open-2", "open-key-1"));

    assert_eq!(first.result(), second.result());
    assert_eq!(second.request_id(), "request-open-2");
    assert!(subscription.receiver.recv().is_ok());
    assert!(subscription.receiver.recv().is_ok());
    assert!(subscription.receiver.try_recv().is_err());
    assert_eq!(
        session
            .subscribe()
            .expect("replacement subscription is available")
            .snapshot
            .last_sequence(),
        2
    );
}

struct BlockingRepository {
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl RepositoryService for BlockingRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        self.entered.wait();
        self.release.wait();
        Ok(RepositoryInspection {
            canonical_root: requested_path.into(),
            display_name: "Kesharon".into(),
        })
    }
}

#[test]
fn cancellation_interrupts_an_in_flight_open_and_preserves_project_state() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let session = session(Arc::new(BlockingRepository {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    }));
    let subscription = session.subscribe().expect("subscription is available");
    let worker_session = Arc::clone(&session);
    let worker = thread::spawn(move || {
        worker_session.dispatch(&open_request("request-open-1", "open-key-1"))
    });

    entered.wait();
    let cancel = ClientRequest::new(
        "request-cancel-1",
        RequestMethod::CancelRequest {
            target_request_id: "request-open-1".into(),
        },
        Some("cancel-key-1".into()),
    )
    .expect("cancel request is valid");
    let cancel_response = session.dispatch(&cancel);
    release.wait();
    let open_response = worker.join().expect("open worker does not panic");

    assert!(matches!(
        cancel_response.result(),
        Some(ResponsePayload::Cancellation {
            outcome: CancellationOutcome::Accepted,
            ..
        })
    ));
    assert_eq!(
        open_response
            .error()
            .map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::OperationCancelled)
    );
    let _started = subscription.receiver.recv().expect("started event");
    assert!(matches!(
        subscription.receiver.recv().expect("cancelled event"),
        StreamMessage::Event { event }
            if matches!(event.payload(), DaemonEventPayload::OperationCancelled { .. })
    ));
    assert!(
        session
            .subscribe()
            .expect("replacement subscription is available")
            .snapshot
            .project()
            .is_none()
    );
}

#[test]
fn an_idempotency_key_rejects_different_payloads_and_in_flight_duplicates() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let session = session(Arc::new(BlockingRepository {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    }));
    let worker_session = Arc::clone(&session);
    let worker = thread::spawn(move || {
        worker_session.dispatch(&open_request("request-open-1", "shared-key"))
    });

    entered.wait();
    let duplicate = session.dispatch(&open_request("request-open-2", "shared-key"));
    let conflict = session.dispatch(&open_request_for(
        "request-open-3",
        "shared-key",
        r"D:\code\different",
    ));
    release.wait();
    let completed = worker.join().expect("open worker does not panic");

    assert!(completed.result().is_some());
    assert_eq!(
        duplicate.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::RequestInProgress)
    );
    assert_eq!(duplicate.request_id(), "request-open-2");
    assert_eq!(
        conflict.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(conflict.request_id(), "request-open-3");
}

#[test]
fn cancel_cannot_reuse_an_in_flight_open_idempotency_key() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let session = session(Arc::new(BlockingRepository {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    }));
    let worker_session = Arc::clone(&session);
    let worker = thread::spawn(move || {
        worker_session.dispatch(&open_request("request-open-1", "shared-key"))
    });

    entered.wait();
    let conflict = session.dispatch(
        &ClientRequest::new(
            "request-cancel-1",
            RequestMethod::CancelRequest {
                target_request_id: "request-open-1".into(),
            },
            Some("shared-key".into()),
        )
        .expect("cancel request is valid"),
    );
    release.wait();
    let completed = worker.join().expect("open worker does not panic");

    assert_eq!(
        conflict.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::InvalidRequest)
    );
    assert!(completed.result().is_some());
}

#[test]
fn a_completed_idempotency_key_rejects_a_different_payload() {
    let session = session(Arc::new(FixedRepository));
    let _completed = session.dispatch(&open_request("request-open-1", "shared-key"));

    let conflict = session.dispatch(&open_request_for(
        "request-open-2",
        "shared-key",
        r"D:\code\different",
    ));

    assert_eq!(
        conflict.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(conflict.request_id(), "request-open-2");
}

struct CountingRepository(AtomicUsize);

impl RepositoryService for CountingRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(RepositoryInspection {
            canonical_root: requested_path.into(),
            display_name: "Kesharon".into(),
        })
    }
}

#[test]
fn completion_ledger_evicts_the_oldest_entry_after_256_mutations() {
    let repository = Arc::new(CountingRepository(AtomicUsize::new(0)));
    let session = session(repository.clone());

    for index in 0..=256 {
        let response = session.dispatch(&open_request_for(
            &format!("request-{index}"),
            &format!("key-{index}"),
            &format!("project-{index}"),
        ));
        assert!(response.result().is_some());
    }
    let reexecuted = session.dispatch(&open_request_for("request-retry", "key-0", "project-0"));

    assert!(reexecuted.result().is_some());
    assert_eq!(repository.0.load(Ordering::Relaxed), 258);
    assert_eq!(reexecuted.request_id(), "request-retry");
}

#[test]
fn a_new_subscriber_atomically_replaces_and_disconnects_the_previous_one() {
    let session = session(Arc::new(FixedRepository));
    let first = session
        .subscribe()
        .expect("first subscription is available");
    let second = session
        .subscribe()
        .expect("replacement subscription is available");

    assert_eq!(first.snapshot, second.snapshot);
    assert_eq!(
        first.receiver.recv_timeout(Duration::from_millis(100)),
        Err(RecvTimeoutError::Disconnected)
    );
}

#[test]
fn subscriber_overflow_disconnects_and_reconnect_snapshot_covers_skipped_events() {
    let session = session(Arc::new(FixedRepository));
    let subscription = session.subscribe().expect("subscription is available");

    for index in 0..33 {
        let response = session.dispatch(&open_request_for(
            &format!("request-{index}"),
            &format!("key-{index}"),
            &format!("project-{index}"),
        ));
        assert!(response.result().is_some());
    }

    assert_eq!(subscription.receiver.try_iter().count(), 64);
    assert_eq!(
        subscription
            .receiver
            .recv_timeout(Duration::from_millis(100)),
        Err(RecvTimeoutError::Disconnected)
    );
    let replacement = session
        .subscribe()
        .expect("replacement subscription is available");
    assert_eq!(replacement.snapshot.last_sequence(), 66);
    assert_eq!(
        replacement
            .snapshot
            .project()
            .map(|project| project.canonical_root.as_str()),
        Some("project-32")
    );
}

#[test]
fn subscription_snapshot_atomically_captures_an_active_operation() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let session = session(Arc::new(BlockingRepository {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    }));
    let worker_session = Arc::clone(&session);
    let worker = thread::spawn(move || {
        worker_session.dispatch(&open_request("request-open-1", "open-key-1"))
    });

    entered.wait();
    let subscription = session.subscribe().expect("subscription is available");
    assert_eq!(subscription.snapshot.last_sequence(), 1);
    assert_eq!(subscription.snapshot.active_operations().len(), 1);
    assert_eq!(
        subscription.snapshot.active_operations()[0].request_id,
        "request-open-1"
    );
    release.wait();
    let _response = worker.join().expect("open worker does not panic");
    assert!(matches!(
        subscription.receiver.recv().expect("terminal event"),
        StreamMessage::Event { event }
            if event.sequence() == 2
                && matches!(event.payload(), DaemonEventPayload::ProjectOpened { .. })
    ));
}

struct SelectiveRepository;

impl RepositoryService for SelectiveRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        if requested_path == "bad" {
            return Err(ApplicationError::Repository("notGitRepository".into()));
        }
        Ok(RepositoryInspection {
            canonical_root: requested_path.into(),
            display_name: "Kesharon".into(),
        })
    }
}

#[test]
fn failed_replacement_emits_one_terminal_event_and_preserves_the_previous_project() {
    let session = session(Arc::new(SelectiveRepository));
    let subscription = session.subscribe().expect("subscription is available");
    let first = session.dispatch(&open_request_for("request-good", "key-good", "good"));
    assert!(first.result().is_some());
    let failed = session.dispatch(&open_request_for("request-bad", "key-bad", "bad"));

    assert_eq!(
        failed.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::NotGitRepository)
    );
    let events: Vec<_> = subscription.receiver.try_iter().collect();
    assert_eq!(events.len(), 4);
    assert!(matches!(
        events.last(),
        Some(StreamMessage::Event { event })
            if event.sequence() == 4
                && matches!(
                    event.payload(),
                    DaemonEventPayload::OperationFailed {
                        code: ErrorCode::NotGitRepository,
                        ..
                    }
                )
    ));
    let snapshot = session
        .subscribe()
        .expect("replacement subscription is available")
        .snapshot;
    assert_eq!(
        snapshot
            .project()
            .map(|project| project.canonical_root.as_str()),
        Some("good")
    );
}
