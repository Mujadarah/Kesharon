use kesharon_domain::{ExecutionMode, ResourceBudget, Task, TaskCheckpoint, TaskId};

#[test]
fn task_tracks_execution_mode() {
    let budget = ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1)
        .expect("the fixture budget is valid");
    let mut task = Task::new(
        TaskId::new("task-mode-1").expect("valid id"),
        "Implement plan mode toggle",
        budget,
    )
    .expect("valid task");

    assert_eq!(task.execution_mode(), ExecutionMode::Plan);

    task.set_execution_mode(ExecutionMode::Act);
    assert_eq!(task.execution_mode(), ExecutionMode::Act);
}

#[test]
fn task_checkpoint_records_rollback_point() {
    let checkpoint = TaskCheckpoint::new(
        "chk-1",
        "Pre-refactor checkpoint",
        "refs/heads/checkpoint-1",
        1_700_000_000_000,
    )
    .expect("valid checkpoint");

    assert_eq!(checkpoint.id(), "chk-1");
    assert_eq!(checkpoint.description(), "Pre-refactor checkpoint");
    assert_eq!(checkpoint.git_ref(), "refs/heads/checkpoint-1");
    assert_eq!(checkpoint.timestamp_millis(), 1_700_000_000_000);
}

#[test]
fn task_checkpoint_rejects_blank_fields() {
    assert!(TaskCheckpoint::new("", "desc", "ref", 100).is_err());
    assert!(TaskCheckpoint::new("chk-1", "", "ref", 100).is_err());
    assert!(TaskCheckpoint::new("chk-1", "desc", "", 100).is_err());
}
