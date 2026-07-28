use kesharon_domain::{
    ResourceBudget, Task, TaskError, TaskId, TaskPlan, TaskState, TaskStep, TaskStepId,
};

fn budget() -> ResourceBudget {
    ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1)
        .expect("the fixture budget is valid")
}

fn plan() -> TaskPlan {
    TaskPlan::new(vec![
        TaskStep::new(
            TaskStepId::new("step-1").expect("the fixture step id is valid"),
            "Run the focused tests",
        )
        .expect("the fixture step is valid"),
    ])
    .expect("the fixture plan is valid")
}

#[test]
fn approved_task_moves_through_execution_and_review() {
    let mut task = Task::new(
        TaskId::new("task-1").expect("the fixture task id is valid"),
        "Add authenticated IPC",
        budget(),
    )
    .expect("the fixture task is valid");

    assert_eq!(task.state(), TaskState::Draft);

    task.start_planning()
        .expect("draft tasks can enter planning");
    assert_eq!(task.state(), TaskState::Planning);

    task.request_approval(plan())
        .expect("planned tasks can request approval");
    assert_eq!(task.state(), TaskState::AwaitingApproval);

    task.approve().expect("awaiting tasks can be approved");
    assert_eq!(task.state(), TaskState::Executing);

    task.pause().expect("executing tasks can pause");
    assert_eq!(task.state(), TaskState::Paused);

    task.resume().expect("paused tasks can resume");
    assert_eq!(task.state(), TaskState::Executing);

    task.submit_for_review()
        .expect("executing tasks can request review");
    assert_eq!(task.state(), TaskState::AwaitingReview);

    task.request_revision()
        .expect("reviewed tasks can return to execution");
    assert_eq!(task.state(), TaskState::Executing);

    task.submit_for_review()
        .expect("revised tasks can request another review");
    task.accept().expect("reviewed tasks can be accepted");
    assert_eq!(task.state(), TaskState::Completed);
}

#[test]
fn rejecting_approval_cancels_without_entering_execution() {
    let mut task = Task::new(
        TaskId::new("task-2").expect("the fixture task id is valid"),
        "Refactor the provider adapter",
        budget(),
    )
    .expect("the fixture task is valid");
    task.start_planning()
        .expect("draft tasks can enter planning");
    task.request_approval(plan())
        .expect("planned tasks can request approval");

    task.reject().expect("awaiting tasks can be rejected");

    assert_eq!(task.state(), TaskState::Cancelled);
    assert_eq!(task.approve(), Err(TaskError::TerminalState));
}

#[test]
fn invalid_transition_is_rejected_without_changing_state() {
    let mut task = Task::new(
        TaskId::new("task-3").expect("the fixture task id is valid"),
        "Inspect repository state",
        budget(),
    )
    .expect("the fixture task is valid");

    let result = task.submit_for_review();

    assert_eq!(
        result,
        Err(TaskError::InvalidTransition {
            from: TaskState::Draft,
            action: "submit_for_review",
        })
    );
    assert_eq!(task.state(), TaskState::Draft);
}

#[test]
fn empty_goal_is_rejected() {
    let result = Task::new(
        TaskId::new("task-4").expect("the fixture task id is valid"),
        "   ",
        budget(),
    );

    assert_eq!(result, Err(TaskError::EmptyGoal));
}

#[test]
fn empty_plan_is_rejected() {
    assert_eq!(TaskPlan::new(Vec::new()), Err(TaskError::EmptyPlan));
}

#[test]
fn identifiers_and_step_descriptions_must_not_be_blank() {
    assert_eq!(TaskId::new("  "), Err(TaskError::EmptyIdentifier));
    assert_eq!(TaskStepId::new(""), Err(TaskError::EmptyIdentifier));
    assert_eq!(
        TaskStep::new(
            TaskStepId::new("step-2").expect("the fixture step id is valid"),
            " ",
        ),
        Err(TaskError::EmptyStepDescription)
    );
}

#[test]
fn resource_budget_rejects_zero_ceilings() {
    assert_eq!(
        ResourceBudget::new(0, 1, 1),
        Err(TaskError::InvalidResourceBudget)
    );
    assert_eq!(
        ResourceBudget::new(1, 0, 1),
        Err(TaskError::InvalidResourceBudget)
    );
    assert_eq!(
        ResourceBudget::new(1, 1, 0),
        Err(TaskError::InvalidResourceBudget)
    );
}
