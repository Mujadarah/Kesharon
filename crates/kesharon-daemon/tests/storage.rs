use kesharon_application::StateRepository;
use kesharon_daemon::SqliteStateRepository;
use kesharon_domain::{
    ExecutionMode, Project, ProjectId, ResourceBudget, Task, TaskCheckpoint, TaskId,
};

#[test]
fn sqlite_storage_persists_and_restores_projects() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory sqlite opens");

    let project = Project::new(
        ProjectId::new("project-sqlite-1").expect("valid id"),
        "Kesharon Agent",
        "/workspace/kesharon",
        true,
    )
    .expect("valid project");

    storage
        .save_project(&project)
        .expect("save project succeeds");

    let loaded = storage
        .load_project("project-sqlite-1")
        .expect("load project query succeeds")
        .expect("project exists");

    assert_eq!(loaded.id().as_str(), "project-sqlite-1");
    assert_eq!(loaded.display_name(), "Kesharon Agent");
    assert_eq!(loaded.canonical_root(), "/workspace/kesharon");
    assert!(loaded.is_trusted());

    let last_active = storage
        .load_last_active_project()
        .expect("load last active succeeds")
        .expect("last active project exists");

    assert_eq!(last_active.id().as_str(), "project-sqlite-1");
}

#[test]
fn sqlite_storage_persists_tasks_checkpoints_and_active_task() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory sqlite opens");

    let project = Project::new(
        ProjectId::new("project-sqlite-2").expect("valid id"),
        "Kesharon Agent",
        "/workspace/kesharon",
        false,
    )
    .expect("valid project");
    storage
        .save_project(&project)
        .expect("save project succeeds");

    let budget = ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 2)
        .expect("valid base budget")
        .with_token_limits(100_000, 20_000, 500_000)
        .expect("valid token limits");

    let mut task = Task::new(
        TaskId::new("task-sqlite-1").expect("valid id"),
        "Implement SQLite storage",
        budget,
    )
    .expect("valid task");
    task.set_execution_mode(ExecutionMode::Act);

    storage.save_task(&task).expect("save task succeeds");

    let loaded_task = storage
        .load_task("task-sqlite-1")
        .expect("load task succeeds")
        .expect("task exists");

    assert_eq!(loaded_task.id().as_str(), "task-sqlite-1");
    assert_eq!(loaded_task.goal(), "Implement SQLite storage");
    assert_eq!(loaded_task.execution_mode(), ExecutionMode::Act);
    assert_eq!(loaded_task.budget().max_prompt_tokens(), Some(100_000));
    assert_eq!(loaded_task.budget().max_cost_micros(), Some(500_000));

    let chk1 = TaskCheckpoint::new(
        "chk-sqlite-1",
        "Initial checkpoint",
        "refs/heads/ckpt-1",
        1_700_000_000_100,
    )
    .expect("valid checkpoint");

    let chk2 = TaskCheckpoint::new(
        "chk-sqlite-2",
        "Second checkpoint",
        "refs/heads/ckpt-2",
        1_700_000_000_200,
    )
    .expect("valid checkpoint");

    storage
        .save_checkpoint("task-sqlite-1", &chk1)
        .expect("save chk1");
    storage
        .save_checkpoint("task-sqlite-1", &chk2)
        .expect("save chk2");

    let checkpoints = storage
        .list_checkpoints("task-sqlite-1")
        .expect("list checkpoints");
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].id(), "chk-sqlite-1");
    assert_eq!(checkpoints[1].id(), "chk-sqlite-2");

    let active_task = storage
        .load_active_task("project-sqlite-2")
        .expect("load active task succeeds")
        .expect("active task exists");
    assert_eq!(active_task.id().as_str(), "task-sqlite-1");
}

#[test]
fn sqlite_storage_clears_active_task_upon_terminal_state() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory sqlite opens");

    let project = Project::new(
        ProjectId::new("project-term-1").expect("valid id"),
        "Terminal Project",
        "/workspace/term",
        true,
    )
    .expect("valid project");
    storage.save_project(&project).expect("saved project");

    let budget =
        ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1).expect("valid budget");
    let mut task = Task::new(
        TaskId::new("task-term-1").expect("valid id"),
        "Cancel me",
        budget,
    )
    .expect("valid task");

    task.start_planning().expect("planning started");
    let plan = kesharon_domain::TaskPlan::new(vec![
        kesharon_domain::TaskStep::new(
            kesharon_domain::TaskStepId::new("step-1").expect("valid id"),
            "First step",
        )
        .expect("valid step"),
    ])
    .expect("valid plan");
    task.request_approval(plan).expect("approval requested");

    storage.save_task(&task).expect("saved active task");
    assert!(
        storage
            .load_active_task("project-term-1")
            .expect("query succeeds")
            .is_some()
    );

    // Reject transitions the task to Cancelled (terminal)
    task.reject().expect("task rejected");
    assert!(task.state().is_terminal());
    assert!(!task.is_active());

    storage.save_task(&task).expect("saved terminal task");

    // Once terminal, load_active_task must return None
    let active_opt = storage
        .load_active_task("project-term-1")
        .expect("query succeeds");
    assert_eq!(active_opt, None);
}

#[test]
fn sqlite_storage_enforces_foreign_keys() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory sqlite opens");

    let chk = TaskCheckpoint::new(
        "chk-orphan-1",
        "Orphan checkpoint",
        "refs/heads/orphan",
        1_700_000_000_000,
    )
    .expect("valid checkpoint");

    // Saving checkpoint for nonexistent task must fail due to FOREIGN KEY constraint
    let result = storage.save_checkpoint("nonexistent-task", &chk);
    assert!(result.is_err());

    let budget =
        ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1).expect("valid budget");
    let task = Task::new(
        TaskId::new("task-no-proj").expect("valid id"),
        "Orphan task",
        budget,
    )
    .expect("valid task");

    // Saving a task without any existing project must fail
    let task_result = storage.save_task(&task);
    assert!(task_result.is_err());
}

#[test]
fn sqlite_storage_records_and_retrieves_idempotency_entries() {
    let mut storage = SqliteStateRepository::in_memory().expect("in-memory sqlite opens");

    storage
        .record_idempotency("key-open-1", "hash-abc", "{\"status\":\"ok\"}")
        .expect("record idempotency");

    let cached = storage
        .find_idempotency("key-open-1")
        .expect("query idempotency")
        .expect("idempotency record found");

    assert_eq!(cached, "{\"status\":\"ok\"}");

    assert_eq!(
        storage
            .find_idempotency("key-unknown")
            .expect("query idempotency"),
        None
    );
}

#[test]
fn sqlite_storage_file_backed_uses_wal_and_persists_across_handles() {
    let dir = std::env::temp_dir().join(format!(
        "kesharon-sqlite-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("valid clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("dir created");
    let db_path = dir.join("state.db");

    {
        let mut storage = SqliteStateRepository::open(&db_path).expect("opens file db");

        // Verify WAL journal mode is active
        let conn = rusqlite::Connection::open(&db_path).expect("verify conn");
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("query journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let project = Project::new(
            ProjectId::new("file-proj-1").expect("valid id"),
            "Persistent Project",
            "/path/to/repo",
            true,
        )
        .expect("valid project");
        storage.save_project(&project).expect("save succeeds");
    }

    {
        let storage = SqliteStateRepository::open(&db_path).expect("reopens file db");
        let loaded = storage
            .load_last_active_project()
            .expect("query succeeds")
            .expect("last active project exists");
        assert_eq!(loaded.id().as_str(), "file-proj-1");
        assert_eq!(loaded.display_name(), "Persistent Project");
    }

    let _ = std::fs::remove_dir_all(&dir);
}
