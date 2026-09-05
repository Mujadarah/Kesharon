use kesharon_domain::{ResourceBudget, TaskError};

#[test]
fn budget_supports_optional_token_and_cost_limits() {
    let budget = ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1)
        .expect("valid base budget")
        .with_token_limits(100_000, 20_000, 500_000)
        .expect("valid token limits");

    assert_eq!(budget.max_prompt_tokens(), Some(100_000));
    assert_eq!(budget.max_completion_tokens(), Some(20_000));
    assert_eq!(budget.max_cost_micros(), Some(500_000));
}

#[test]
fn budget_rejects_zero_token_limits_when_configured() {
    let base =
        ResourceBudget::new(650 * 1024 * 1024, 128 * 1024 * 1024, 1).expect("valid base budget");

    assert_eq!(
        base.with_token_limits(0, 20_000, 500_000),
        Err(TaskError::InvalidResourceBudget)
    );
    assert_eq!(
        base.with_token_limits(100_000, 0, 500_000),
        Err(TaskError::InvalidResourceBudget)
    );
    assert_eq!(
        base.with_token_limits(100_000, 20_000, 0),
        Err(TaskError::InvalidResourceBudget)
    );
}
