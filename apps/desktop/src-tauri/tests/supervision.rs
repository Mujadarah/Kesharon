use kesharon_desktop_host::{
    ExitDecision, LaunchConfiguration, LifecycleAction, RestartBudget, lifecycle_action,
};
use kesharon_protocol::LaunchToken;

#[test]
fn launch_configuration_uses_an_ephemeral_256_bit_token_and_unique_endpoint() {
    let first = LaunchConfiguration::generate(std::env::temp_dir())
        .expect("a launch configuration can be generated");
    let second = LaunchConfiguration::generate(std::env::temp_dir())
        .expect("a second launch configuration can be generated");

    assert!(LaunchToken::parse_hex(first.launch_token()).is_ok());
    assert_ne!(first.launch_token(), second.launch_token());
    assert_ne!(first.endpoint(), second.endpoint());
}

#[test]
fn an_unexpected_daemon_exit_restarts_only_once() {
    let mut budget = RestartBudget::default();

    assert_eq!(budget.record_unexpected_exit(), ExitDecision::Restart);
    assert_eq!(
        budget.record_unexpected_exit(),
        ExitDecision::MarkRecoverable
    );
    assert_eq!(
        budget.record_unexpected_exit(),
        ExitDecision::MarkRecoverable
    );
}

#[test]
fn closing_the_window_keeps_the_agent_alive_until_explicit_quit() {
    assert_eq!(lifecycle_action(false), LifecycleAction::HideToTray);
    assert_eq!(lifecycle_action(true), LifecycleAction::Exit);
}
