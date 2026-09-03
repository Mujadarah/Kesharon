use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_desktop_host::commands::{
    execute_cancel_request, execute_health, execute_open_project,
};
use kesharon_desktop_host::{DaemonClient, DaemonState};
use kesharon_ipc::{LocalEndpoint, LocalServer};
use kesharon_protocol::{
    CancellationOutcome, HealthStatus, LaunchToken, PROTOCOL_VERSION, ResponsePayload,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(0);

fn unique_endpoint() -> LocalEndpoint {
    let endpoint_id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    let value = format!("kesharon-cmd-test-{}-{}", std::process::id(), endpoint_id);

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "k-cmd-{:x}-{endpoint_id:x}.sock",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

fn create_temp_git_repository() -> std::path::PathBuf {
    let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let repo_dir =
        std::env::temp_dir().join(format!("kesharon-cmd-repo-{}-{}", std::process::id(), id));
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("the test repo dir is creatable");
    repo_dir
}

#[test]
fn daemon_commands_propagate_open_and_cancel_requests() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
        let daemon =
            Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
        let runtime = ServerRuntime::new(daemon);

        let server_endpoint = endpoint.clone();
        let _server_thread = thread::spawn(move || {
            let _ = runtime.run(&server);
        });

        let client =
            DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(3))
                .expect("the client configuration is valid");

        let daemon_state = DaemonState::new_with_client(client);

        // Test daemon_health
        let health_response = execute_health(&daemon_state)
            .await
            .expect("health command succeeds");
        assert!(matches!(
            health_response.result(),
            Some(ResponsePayload::Health {
                status: HealthStatus::Ready,
                protocol_version: PROTOCOL_VERSION,
            })
        ));

        // Test daemon_open_project
        let repo_dir = create_temp_git_repository();
        let repo_path = repo_dir.to_str().expect("valid path str").to_owned();

        let open_response = execute_open_project(&daemon_state, repo_path, None, None)
            .await
            .expect("open command succeeds");
        assert!(matches!(
            open_response.result(),
            Some(ResponsePayload::ProjectOpened { .. })
        ));

        // Test daemon_cancel_request
        let cancel_response =
            execute_cancel_request(&daemon_state, "nonexistent-id".into(), None, None)
                .await
                .expect("cancel command succeeds");
        assert!(matches!(
            cancel_response.result(),
            Some(ResponsePayload::Cancellation {
                outcome: CancellationOutcome::NotFound,
                ..
            })
        ));

        let _ = std::fs::remove_dir_all(&repo_dir);
    });
}

#[test]
fn daemon_commands_fail_gracefully_when_uninitialized() {
    tauri::async_runtime::block_on(async {
        let daemon_state = DaemonState::default();

        let open_err = execute_open_project(&daemon_state, "dummy/path".into(), None, None).await;
        assert!(open_err.is_err());

        let cancel_err = execute_cancel_request(&daemon_state, "target".into(), None, None).await;
        assert!(cancel_err.is_err());
    });
}
