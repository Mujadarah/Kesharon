use std::sync::{Arc, Mutex};

use kesharon_application::{IdGenerator, RepositoryInspection, RepositoryService, StateRepository};
use kesharon_daemon::{DaemonSession, SharedStateRepository, SqliteStateRepository};
use kesharon_domain::{Project, ProjectId};
use kesharon_protocol::{ClientRequest, RequestMethod, ResponsePayload};

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

#[test]
fn daemon_session_recovers_last_active_project_on_startup() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory db");
    let project = Project::new(
        ProjectId::new("proj-recovered-42").expect("valid id"),
        "My Persisted Project",
        "D:\\repos\\persisted",
        true,
    )
    .expect("valid project");
    storage.save_project(&project).expect("saved project");

    let storage_boxed: Box<dyn StateRepository + Send + Sync> = Box::new(storage);
    let storage_arc: SharedStateRepository = Arc::new(Mutex::new(storage_boxed));

    let session = DaemonSession::with_storage(
        "stream-recovery-1",
        Arc::new(StubRepository),
        Arc::new(FixedId),
        Some(storage_arc),
    );

    let subscription = session.subscribe().expect("subscription succeeds");
    let recovered_project = subscription
        .snapshot
        .project()
        .expect("project is recovered");
    assert_eq!(recovered_project.id, "proj-recovered-42");
    assert_eq!(recovered_project.display_name, "My Persisted Project");
    assert_eq!(recovered_project.canonical_root, "D:\\repos\\persisted");
    assert!(recovered_project.trusted);
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
        );

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
        );

        let subscription = session.subscribe().expect("subscription succeeds");
        let project = subscription.snapshot.project().expect("project recovered");
        assert_eq!(project.id, "project-fixed-1");
        assert_eq!(project.display_name, "test-recovered-project");
        assert_eq!(project.canonical_root, "D:\\repos\\new-project");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
