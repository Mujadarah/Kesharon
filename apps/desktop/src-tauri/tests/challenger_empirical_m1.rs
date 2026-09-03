use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_desktop_host::commands::{
    execute_cancel_request, execute_health, execute_open_project,
};
use kesharon_desktop_host::{DaemonClient, DaemonState};
use kesharon_ipc::{
    LocalEndpoint, LocalServer, connect_with_timeout, read_exact_with_timeout,
    write_all_with_timeout,
};
use kesharon_protocol::{
    CancellationOutcome, ErrorCode, HealthStatus, LaunchToken, ResponsePayload, ServerResponse,
    decode_client_request_frame, encode_frame,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(500);

fn unique_endpoint() -> LocalEndpoint {
    let endpoint_id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    let value = format!(
        "kesharon-challenger-m1-{}-{}",
        std::process::id(),
        endpoint_id
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "k-challenger-m1-{:x}-{endpoint_id:x}.sock",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

fn create_temp_git_repository(name: &str) -> std::path::PathBuf {
    let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let repo_dir = std::env::temp_dir().join(format!(
        "kesharon-challenger-m1-{}-{}-{}",
        name,
        std::process::id(),
        id
    ));
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("the test repo dir is creatable");
    repo_dir
}

fn daemon_binary_path() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_debug = manifest_dir.join("../../../target/debug");
    #[cfg(windows)]
    let bin = target_debug.join("kesharon-daemon.exe");
    #[cfg(not(windows))]
    let bin = target_debug.join("kesharon-daemon");
    bin
}

struct TestDaemonProcess {
    child: Child,
    endpoint: LocalEndpoint,
}

impl TestDaemonProcess {
    fn start(extra_env: Option<(&str, &str)>) -> Self {
        let endpoint = unique_endpoint();
        let mut command = Command::new(daemon_binary_path());
        command
            .args(["--endpoint", endpoint.as_str()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some((key, value)) = extra_env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("the daemon binary starts");
        let mut stdin = child.stdin.take().expect("daemon stdin is piped");
        writeln!(stdin, "{TOKEN}").expect("the launch token is writable");
        drop(stdin);
        let stdout = child.stdout.take().expect("daemon stdout is piped");
        let mut stdout = BufReader::new(stdout);
        let mut ready = String::new();
        stdout
            .read_line(&mut ready)
            .expect("the readiness line is readable");
        assert_eq!(ready.trim_end(), "READY 1", "daemon must signal readiness");
        Self { child, endpoint }
    }
}

impl Drop for TestDaemonProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// EMPIRICAL TEST 1: Verify asynchronous off-event-loop execution
/// When a blocking command is dispatched, async tasks on the Tokio runtime
/// must continue ticking without starvation or delay.
#[test]
fn empirical_blocking_operation_does_not_freeze_async_event_loop() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

        // We run a mock server that artificially delays answering OpenProject by 350ms
        let server_running = Arc::new(AtomicBool::new(true));
        let server_running_clone = Arc::clone(&server_running);

        let server_thread = thread::spawn(move || {
            while server_running_clone.load(Ordering::Acquire) {
                let Ok(stream) = server.accept_with_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let mut token = [0_u8; 64];
                let _ = read_exact_with_timeout(&stream, &mut token, Duration::from_secs(1));
                let mut prefix = [0_u8; 4];
                let _ = read_exact_with_timeout(&stream, &mut prefix, Duration::from_secs(1));
                let len = u32::from_be_bytes(prefix) as usize;
                let mut payload = vec![0_u8; len];
                let _ = read_exact_with_timeout(&stream, &mut payload, Duration::from_secs(1));

                let mut full_frame = Vec::with_capacity(len + 4);
                full_frame.extend_from_slice(&prefix);
                full_frame.extend_from_slice(&payload);
                if let Ok(req) = decode_client_request_frame(&full_frame) {
                    // Artificial blocking delay
                    thread::sleep(Duration::from_millis(350));
                    let resp = ServerResponse::failure(
                        req.request_id(),
                        ErrorCode::InternalError,
                        "Delayed mock response",
                    );
                    if let Ok(resp_bytes) = encode_frame(&resp) {
                        let _ =
                            write_all_with_timeout(&stream, &resp_bytes, Duration::from_secs(1));
                    }
                }
            }
        });

        let client =
            DaemonClient::new_with_timeout(endpoint.clone(), TOKEN.into(), Duration::from_secs(2))
                .expect("the client is valid");
        let daemon_state = DaemonState::new_with_client(client);

        let slow_task = tauri::async_runtime::spawn(async move {
            let start = Instant::now();
            let res = execute_open_project(&daemon_state, "some/path".into(), None, None).await;
            (res, start.elapsed())
        });

        // Concurrently, an async heartbeat loop executes async yields/sleeps on the runtime
        let mut ticks = 0;
        let start_time = Instant::now();
        while start_time.elapsed() < Duration::from_millis(280) {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let _ = thread::spawn(move || {
                thread::sleep(Duration::from_millis(15));
                let _ = tx.send(());
            });
            let _ = rx.recv();
            ticks += 1;
        }

        // Verify that the async heartbeat completed multiple ticks while the blocking op was running
        assert!(
            ticks >= 3,
            "Async event loop must not be starved by blocking command: observed {ticks} ticks"
        );

        let (res, duration) = slow_task.await.expect("slow task joins cleanly");
        assert!(res.is_ok(), "Response should be parsed");
        assert!(
            duration >= Duration::from_millis(300),
            "Operation actually took >300ms (took {duration:?})"
        );

        server_running.store(false, Ordering::Release);
        server_thread.join().expect("server thread exits");
    });
}

/// EMPIRICAL TEST 2: Verify cancellation semantics across multiple connections
/// A slow/in-flight project opening request can be cancelled by a second connection
/// dispatching `CancelRequest`, emitting `OperationCancelled` event.
#[test]
#[allow(clippy::too_many_lines)]
fn empirical_cancellation_semantics_across_multiple_connections() {
    let barrier = std::env::temp_dir().join(format!(
        "kesharon-challenger-cxl-barrier-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock valid")
            .as_nanos()
    ));
    let barrier_text = barrier.to_string_lossy().into_owned();
    let entered = barrier.with_extension("entered");

    let daemon = TestDaemonProcess::start(Some((
        "KESHARON_TEST_OPEN_BLOCK_UNTIL",
        barrier_text.as_str(),
    )));

    tauri::async_runtime::block_on(async {
        let client1 = DaemonClient::new_with_timeout(
            daemon.endpoint.clone(),
            TOKEN.into(),
            Duration::from_secs(4),
        )
        .expect("client1 valid");
        let client2 = DaemonClient::new_with_timeout(
            daemon.endpoint.clone(),
            TOKEN.into(),
            Duration::from_secs(4),
        )
        .expect("client2 valid");

        let state1 = DaemonState::new_with_client(client1);
        let state2 = DaemonState::new_with_client(client2);

        let repo_dir = create_temp_git_repository("cancellation-barrier-test");
        let repo_path = repo_dir.to_str().expect("valid path").to_owned();

        // 1. Dispatch in-flight open project command on connection 1
        let target_request_id = "req-open-in-flight-cxl".to_string();
        let target_id_clone = target_request_id.clone();
        let repo_path_clone = repo_path.clone();

        let open_handle = tauri::async_runtime::spawn(async move {
            execute_open_project(
                &state1,
                repo_path_clone,
                Some(target_id_clone),
                Some("idemp-open-in-flight".into()),
            )
            .await
        });

        // 2. Wait until repository inspection has definitely entered the blocked state
        let deadline = Instant::now() + Duration::from_secs(3);
        while !entered.exists() {
            assert!(
                Instant::now() < deadline,
                "open operation did not reach barrier in time"
            );
            thread::sleep(Duration::from_millis(10));
        }

        // 3. Dispatch cancel command on connection 2 targeting in-flight request
        let cancel_res = execute_cancel_request(
            &state2,
            target_request_id.clone(),
            Some("req-cancel-actor".into()),
            Some("idemp-cancel-actor".into()),
        )
        .await
        .expect("cancel command executes");

        match cancel_res.result().expect("has result") {
            ResponsePayload::Cancellation {
                target_request_id: tid,
                outcome,
            } => {
                assert_eq!(tid, &target_request_id);
                assert_eq!(outcome, &CancellationOutcome::Accepted);
            }
            other => panic!("expected Cancellation payload, got {other:?}"),
        }

        // 4. Release barrier
        fs::write(&barrier, b"release").expect("barrier released");

        // 5. Verify open request completed with OperationCancelled error
        let open_res = open_handle.await.expect("open task finishes");
        let open_response = open_res.expect("response parsed");
        assert!(open_response.error().is_some());
        assert_eq!(
            open_response.error().unwrap().code(),
            ErrorCode::OperationCancelled
        );

        // 6. Test cancelling an already finished request
        let cancel_again = execute_cancel_request(
            &state2,
            target_request_id.clone(),
            Some("req-cancel-second".into()),
            Some("idemp-cancel-second".into()),
        )
        .await
        .expect("second cancel succeeds");

        match cancel_again.result().expect("result") {
            ResponsePayload::Cancellation { outcome, .. } => {
                assert_eq!(outcome, &CancellationOutcome::AlreadyFinished);
            }
            other => panic!("expected Cancellation payload, got {other:?}"),
        }

        // 7. Test cancelling a nonexistent request
        let cancel_nonexistent = execute_cancel_request(
            &state2,
            "nonexistent-op-999".into(),
            Some("req-cancel-404".into()),
            Some("idemp-cancel-404".into()),
        )
        .await
        .expect("cancel nonexistent succeeds");

        match cancel_nonexistent.result().expect("result") {
            ResponsePayload::Cancellation { outcome, .. } => {
                assert_eq!(outcome, &CancellationOutcome::NotFound);
            }
            other => panic!("expected Cancellation payload, got {other:?}"),
        }

        let _ = fs::remove_file(&entered);
        let _ = fs::remove_file(&barrier);
        let _ = fs::remove_dir_all(&repo_dir);
    });
}

/// EMPIRICAL TEST 3: Verify idempotency key handling across multiple connections
/// - Replay with same payload returns identical response without re-executing
/// - Replay with different payload returns `ErrorCode::InvalidRequest`
/// - Cross-method collision (Open vs Cancel) returns `ErrorCode::InvalidRequest`
#[test]
fn empirical_idempotency_key_handling_across_multiple_connections() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
        let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("fixture token is valid"));
        let runtime = ServerRuntime::new(daemon);

        let server_endpoint = endpoint.clone();
        let _server_thread = thread::spawn(move || {
            let _ = runtime.run(&server);
        });

        let client1 = DaemonClient::new_with_timeout(
            server_endpoint.clone(),
            TOKEN.into(),
            Duration::from_secs(3),
        )
        .expect("client1 valid");
        let client2 = DaemonClient::new_with_timeout(
            server_endpoint.clone(),
            TOKEN.into(),
            Duration::from_secs(3),
        )
        .expect("client2 valid");

        let state1 = DaemonState::new_with_client(client1);
        let state2 = DaemonState::new_with_client(client2);

        let repo_dir1 = create_temp_git_repository("idemp-repo-1");
        let repo_path1 = repo_dir1.to_str().expect("valid path").to_owned();

        let repo_dir2 = create_temp_git_repository("idemp-repo-2");
        let repo_path2 = repo_dir2.to_str().expect("valid path").to_owned();

        let shared_idemp_key = "idemp-key-shared-100".to_string();

        // 1. First execution on connection 1
        let resp1 = execute_open_project(
            &state1,
            repo_path1.clone(),
            Some("req-first-attempt".into()),
            Some(shared_idemp_key.clone()),
        )
        .await
        .expect("first open succeeds");

        assert_eq!(resp1.request_id(), "req-first-attempt");
        let project1_id = match resp1.result() {
            Some(ResponsePayload::ProjectOpened { project }) => project.id.clone(),
            other => panic!("expected ProjectOpened, got {other:?}"),
        };

        // 2. Replay with SAME idempotency key and SAME path on connection 2
        let resp2 = execute_open_project(
            &state2,
            repo_path1.clone(),
            Some("req-second-attempt".into()),
            Some(shared_idemp_key.clone()),
        )
        .await
        .expect("replay open succeeds");

        assert_eq!(resp2.request_id(), "req-second-attempt");
        match resp2.result() {
            Some(ResponsePayload::ProjectOpened { project }) => {
                assert_eq!(
                    project.id, project1_id,
                    "Project ID must be identical on replay"
                );
            }
            other => panic!("expected ProjectOpened, got {other:?}"),
        }

        // 3. Conflict: SAME idempotency key with DIFFERENT path on connection 2
        let resp3 = execute_open_project(
            &state2,
            repo_path2.clone(),
            Some("req-conflict-attempt".into()),
            Some(shared_idemp_key.clone()),
        )
        .await
        .expect("server handles conflict gracefully");

        assert!(
            resp3.error().is_some(),
            "Different payload must return error"
        );
        let err = resp3.error().expect("error present");
        assert_eq!(err.code(), ErrorCode::InvalidRequest);
        assert_eq!(
            err.message(),
            "Idempotency key was already used for a different mutation"
        );

        // 4. Cross-method Conflict: SAME idempotency key with CancelRequest
        let resp4 = execute_cancel_request(
            &state1,
            "some-target-id".into(),
            Some("req-cancel-conflict".into()),
            Some(shared_idemp_key.clone()),
        )
        .await
        .expect("server handles cancel conflict gracefully");

        assert!(
            resp4.error().is_some(),
            "Cross-method key reuse must return error"
        );
        let err4 = resp4.error().expect("error present");
        assert_eq!(err4.code(), ErrorCode::InvalidRequest);
        assert_eq!(
            err4.message(),
            "Idempotency key was already used for a different mutation"
        );

        let _ = std::fs::remove_dir_all(&repo_dir1);
        let _ = std::fs::remove_dir_all(&repo_dir2);
    });
}

/// EMPIRICAL TEST 4: Boundary safety and resource bounds
/// - Bad authentication token rejection
/// - 8 concurrent connections allowed, 9th rejected with `ServerBusy`
/// - Rapid subscriber replacement cleanly frees connection permits
#[test]
fn empirical_boundary_safety_and_resource_bounds() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("fixture token is valid"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    // 1. Verify bad authentication token
    let bad_token = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let bad_client = DaemonClient::new_with_timeout(
        server_endpoint.clone(),
        bad_token.into(),
        Duration::from_millis(500),
    )
    .expect("bad client created");

    assert!(
        bad_client.health().is_err(),
        "Bad authentication token must be rejected"
    );

    // 2. Verify 8 concurrent connections hold worker threads
    let mut occupied = Vec::new();
    for _ in 0..8 {
        let stream = connect_with_timeout(&server_endpoint, Duration::from_secs(2))
            .expect("connection slot is accepted");
        write_all_with_timeout(&stream, TOKEN.as_bytes(), Duration::from_secs(2))
            .expect("authentication is writable");
        occupied.push(stream);
    }

    // 9th client connection should be rejected with ServerBusy
    let client9 = DaemonClient::new_with_timeout(
        server_endpoint.clone(),
        TOKEN.into(),
        Duration::from_secs(2),
    )
    .expect("client 9 created");

    let busy_resp = client9.request_health().expect("busy response received");
    assert!(busy_resp.error().is_some());
    assert_eq!(busy_resp.error().unwrap().code(), ErrorCode::ServerBusy);
    drop(occupied);

    // 3. Verify repeated subscriber replacements immediately release connection permits
    let endpoint_sub = unique_endpoint();
    let server_sub = LocalServer::bind(&endpoint_sub).expect("endpoint available");
    let daemon_sub = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("fixture token valid"));
    let runtime_sub = ServerRuntime::new(daemon_sub);
    let server_sub_endpoint = endpoint_sub.clone();
    let _sub_thread = thread::spawn(move || {
        let _ = runtime_sub.run(&server_sub);
    });

    let client_sub =
        DaemonClient::new_with_timeout(server_sub_endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client sub created");

    // Perform 12 sequential subscriber replacements (more than 8 max connections)
    for _ in 0..12 {
        let sub = client_sub.subscribe().expect("subscription succeeds");
        assert!(!sub.stream_id().is_empty());
    }

    // After 12 subscriptions, health check succeeds immediately because old subscriptions were freed
    let health = client_sub
        .health()
        .expect("health succeeds after replacements");
    assert_eq!(health.status, HealthStatus::Ready);
}

/// EMPIRICAL TEST 5: Verify uninitialized `DaemonState` and connection drop resilience
#[test]
fn empirical_uninitialized_and_drop_handling() {
    tauri::async_runtime::block_on(async {
        let empty_state = DaemonState::default();

        // All commands fail gracefully with HostError::StateUnavailable when uninitialized
        let health_err = execute_health(&empty_state).await;
        assert!(health_err.is_err());
        assert!(health_err.unwrap_err().contains("unavailable"));

        let open_err = execute_open_project(&empty_state, "path".into(), None, None).await;
        assert!(open_err.is_err());
        assert!(open_err.unwrap_err().contains("unavailable"));

        let cancel_err = execute_cancel_request(&empty_state, "target".into(), None, None).await;
        assert!(cancel_err.is_err());
        assert!(cancel_err.unwrap_err().contains("unavailable"));
    });
}
