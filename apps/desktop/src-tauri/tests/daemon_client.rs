use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_desktop_host::DaemonClient;
use kesharon_ipc::{LocalEndpoint, LocalServer};
use kesharon_protocol::{
    CancellationOutcome, ClientRequest, DaemonEventPayload, HealthStatus, LaunchToken,
    OperationKind, PROTOCOL_VERSION, RequestMethod, ResponsePayload,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(0);

fn unique_endpoint() -> LocalEndpoint {
    let endpoint_id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    let value = format!("kesharon-host-test-{}-{}", std::process::id(), endpoint_id);

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!("k-{:x}-{endpoint_id:x}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

fn create_temp_git_repository() -> std::path::PathBuf {
    let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let repo_dir =
        std::env::temp_dir().join(format!("kesharon-test-repo-{}-{}", std::process::id(), id));
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("the test repo dir is creatable");
    repo_dir
}

#[test]
fn client_authenticates_and_decodes_daemon_health() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let server_thread = thread::spawn(move || {
        daemon
            .serve_local_connection(&server)
            .expect("the client request is valid");
    });
    let client =
        DaemonClient::new(endpoint, TOKEN.into()).expect("the client configuration is valid");

    let health = client.health().expect("the daemon returns health");

    assert_eq!(health.status, HealthStatus::Ready);
    assert_eq!(health.protocol_version, PROTOCOL_VERSION);
    server_thread
        .join()
        .expect("the server thread does not panic");
}

#[test]
fn client_deadline_bounds_a_nonresponding_daemon() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("the client connects");
        let mut token = [0_u8; 64];
        stream
            .read_exact(&mut token)
            .expect("the token is readable");
        let mut prefix = [0_u8; 4];
        stream
            .read_exact(&mut prefix)
            .expect("the frame prefix is readable");
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        stream
            .read_exact(&mut payload)
            .expect("the frame payload is readable");
        thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"late");
    });
    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(40))
        .expect("the client configuration is valid");
    let started = Instant::now();

    assert!(client.health().is_err());
    assert!(started.elapsed() < Duration::from_millis(200));
    server_thread
        .join()
        .expect("the server thread does not panic");
}

#[test]
fn client_send_request_dispatches_arbitrary_client_request() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let server_thread = thread::spawn(move || {
        daemon
            .serve_local_connection(&server)
            .expect("the client request is valid");
    });
    let client =
        DaemonClient::new(endpoint, TOKEN.into()).expect("the client configuration is valid");

    let request = ClientRequest::new("custom-req-1", RequestMethod::Health, None)
        .expect("valid client request");
    let response = client
        .send_request(&request)
        .expect("response received successfully");

    assert_eq!(response.request_id(), "custom-req-1");
    assert!(matches!(
        response.result(),
        Some(ResponsePayload::Health {
            status: HealthStatus::Ready,
            protocol_version: PROTOCOL_VERSION,
        })
    ));
    server_thread
        .join()
        .expect("the server thread does not panic");
}

#[test]
fn client_open_project_dispatches_and_returns_project_snapshot() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let server_thread = thread::spawn(move || {
        daemon
            .serve_local_connection(&server)
            .expect("the client request is valid");
    });
    let client =
        DaemonClient::new(endpoint, TOKEN.into()).expect("the client configuration is valid");

    let repo_dir = create_temp_git_repository();
    let repo_path = repo_dir.to_str().expect("valid path str");

    let response = client
        .open_project(repo_path, "req-open-1", "idemp-open-1")
        .expect("open_project succeeds");

    assert_eq!(response.request_id(), "req-open-1");
    match response.result() {
        Some(ResponsePayload::ProjectOpened { project }) => {
            assert!(!project.trusted);
            assert!(!project.id.is_empty());
            assert!(!project.display_name.is_empty());
        }
        other => panic!("expected ProjectOpened payload, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&repo_dir);
    server_thread
        .join()
        .expect("the server thread does not panic");
}

#[test]
fn client_cancel_request_dispatches_and_returns_cancellation_outcome() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let server_thread = thread::spawn(move || {
        daemon
            .serve_local_connection(&server)
            .expect("the client request is valid");
    });
    let client =
        DaemonClient::new(endpoint, TOKEN.into()).expect("the client configuration is valid");

    let response = client
        .cancel_request("target-op-404", "req-cancel-1", "idemp-cancel-1")
        .expect("cancel_request succeeds");

    assert_eq!(response.request_id(), "req-cancel-1");
    match response.result() {
        Some(ResponsePayload::Cancellation {
            target_request_id,
            outcome,
        }) => {
            assert_eq!(target_request_id, "target-op-404");
            assert_eq!(outcome, &CancellationOutcome::NotFound);
        }
        other => panic!("expected Cancellation payload, got {other:?}"),
    }

    server_thread
        .join()
        .expect("the server thread does not panic");
}

#[test]
fn client_subscribe_receives_snapshot_and_live_events() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("the client configuration is valid");

    let subscription = client.subscribe().expect("subscription handshake succeeds");

    assert!(!subscription.stream_id().is_empty());
    assert_eq!(subscription.initial_snapshot().last_sequence(), 0);
    assert!(subscription.initial_snapshot().project().is_none());

    let repo_dir = create_temp_git_repository();
    let repo_path = repo_dir.to_str().expect("valid path str");

    let client_clone = client.clone();
    let path_clone = repo_path.to_owned();
    let open_thread = thread::spawn(move || {
        client_clone
            .open_project(&path_clone, "req-open-live", "idemp-open-live")
            .expect("open_project succeeds");
    });

    let first_event = subscription
        .read_next_event(Duration::from_secs(3))
        .expect("operation started event received");
    assert_eq!(first_event.sequence(), 1);
    assert!(matches!(
        first_event.payload(),
        DaemonEventPayload::OperationStarted {
            kind: OperationKind::OpenProject,
            ..
        }
    ));

    let second_event = subscription
        .read_next_event(Duration::from_secs(3))
        .expect("project opened event received");
    assert_eq!(second_event.sequence(), 2);
    assert!(matches!(
        second_event.payload(),
        DaemonEventPayload::ProjectOpened { .. }
    ));

    open_thread.join().expect("open thread finishes cleanly");
    let _ = std::fs::remove_dir_all(&repo_dir);
}
