#![allow(clippy::too_many_lines, clippy::cast_possible_truncation)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_desktop_host::commands::{execute_open_project, execute_subscribe_daemon_events};
use kesharon_desktop_host::{
    DaemonClient, DaemonState, DaemonStreamUpdate, INITIAL_BACKOFF, MAX_BACKOFF, next_backoff,
};
use kesharon_ipc::{LocalEndpoint, LocalServer, read_exact_with_timeout, write_all_with_timeout};
use kesharon_protocol::{
    DaemonEvent, DaemonEventPayload, LaunchToken, OperationKind, ResponsePayload, ServerResponse,
    StreamMessage, WorkspaceSnapshot, encode_frame,
};
use tauri::ipc::Channel;

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(2000);

fn unique_endpoint() -> LocalEndpoint {
    let endpoint_id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    let value = format!(
        "kesharon-recovery-test-{}-{}",
        std::process::id(),
        endpoint_id
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "k-recovery-{:x}-{endpoint_id:x}.sock",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

fn create_temp_git_repository(name: &str) -> std::path::PathBuf {
    let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let repo_dir = std::env::temp_dir().join(format!(
        "kesharon-recovery-repo-{}-{}-{}",
        name,
        std::process::id(),
        id
    ));
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("the test repo dir is creatable");
    repo_dir
}

fn create_test_channel() -> (
    Channel<DaemonStreamUpdate>,
    std::sync::mpsc::Receiver<DaemonStreamUpdate>,
) {
    let (tx, rx) = channel();
    let channel = Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(json) = body {
            let update = parse_daemon_stream_update(&json);
            let _ = tx.send(update);
        }
        Ok(())
    });
    (channel, rx)
}

fn parse_daemon_stream_update(json: &str) -> DaemonStreamUpdate {
    if json.contains("\"type\":\"reconnecting\"") {
        return DaemonStreamUpdate::Reconnecting;
    }
    if json.contains("\"type\":\"unavailable\"") {
        return DaemonStreamUpdate::Unavailable {
            message: "unavailable".into(),
        };
    }
    if let Some(msg_idx) = json.find("\"message\":") {
        let msg_slice = json[msg_idx + 10..].trim();
        let msg_json = msg_slice.strip_suffix('}').unwrap_or(msg_slice);
        let bytes = msg_json.as_bytes();
        let len = bytes.len() as u32;
        let mut frame = Vec::with_capacity(bytes.len() + 4);
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(bytes);
        if let Ok(message) = kesharon_protocol::decode_stream_message_frame(&frame) {
            return DaemonStreamUpdate::Message { message };
        }
    }
    panic!("unrecognized DaemonStreamUpdate JSON: {json}");
}

// 1. Stream worker connects and forwards initial snapshot and live events
#[test]
fn stream_worker_forwards_initial_snapshot_and_live_events() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("endpoint available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");
    let daemon_state = DaemonState::new_with_client(client);

    let (channel, rx) = create_test_channel();
    execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

    // First message must be initial WorkspaceSnapshot
    let first = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("initial snapshot received");
    match first {
        DaemonStreamUpdate::Message {
            message: StreamMessage::Snapshot { snapshot },
        } => {
            assert_eq!(snapshot.last_sequence(), 0);
            assert!(snapshot.project().is_none());
        }
        other => panic!("expected Initial Snapshot, got {other:?}"),
    }

    // Trigger live operation
    let repo_dir = create_temp_git_repository("live-events");
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    let open_res = tauri::async_runtime::block_on(execute_open_project(
        &daemon_state,
        repo_path,
        Some("req-open-stream".into()),
        Some("idemp-open-stream".into()),
    ))
    .expect("open_project succeeds");
    assert!(matches!(
        open_res.result(),
        Some(ResponsePayload::ProjectOpened { .. })
    ));

    // Expect OperationStarted event
    let second = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("operation started received");
    match second {
        DaemonStreamUpdate::Message {
            message: StreamMessage::Event { event },
        } => {
            assert_eq!(event.sequence(), 1);
            assert!(matches!(
                event.payload(),
                DaemonEventPayload::OperationStarted {
                    kind: OperationKind::OpenProject,
                    ..
                }
            ));
        }
        other => panic!("expected OperationStarted event, got {other:?}"),
    }

    // Expect ProjectOpened event
    let third = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("project opened received");
    match third {
        DaemonStreamUpdate::Message {
            message: StreamMessage::Event { event },
        } => {
            assert_eq!(event.sequence(), 2);
            assert!(matches!(
                event.payload(),
                DaemonEventPayload::ProjectOpened { .. }
            ));
        }
        other => panic!("expected ProjectOpened event, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&repo_dir);
}

// 2. Exponential backoff progression unit verification
#[test]
fn stream_worker_exponential_backoff_doubling_progression() {
    assert_eq!(INITIAL_BACKOFF, Duration::from_millis(50));
    assert_eq!(MAX_BACKOFF, Duration::from_secs(1));

    let b1 = INITIAL_BACKOFF; // 50ms
    let b2 = next_backoff(b1); // 100ms
    assert_eq!(b2, Duration::from_millis(100));

    let b3 = next_backoff(b2); // 200ms
    assert_eq!(b3, Duration::from_millis(200));

    let b4 = next_backoff(b3); // 400ms
    assert_eq!(b4, Duration::from_millis(400));

    let b5 = next_backoff(b4); // 800ms
    assert_eq!(b5, Duration::from_millis(800));

    let b6 = next_backoff(b5); // 1000ms (capped at 1s)
    assert_eq!(b6, Duration::from_secs(1));

    let b7 = next_backoff(b6); // 1000ms
    assert_eq!(b7, Duration::from_secs(1));
}

// 3. Stream disconnect triggers exponential backoff and authoritative snapshot reload upon reconnect
#[test]
fn stream_worker_reconnects_with_exponential_backoff_on_disconnect() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");

        let disconnect_flag = Arc::new(AtomicBool::new(false));
        let disconnect_flag_clone = Arc::clone(&disconnect_flag);

        let server_thread = thread::spawn(move || {
            // First connection: accept and disconnect after initial snapshot
            let stream1 = server.accept().expect("client connects first time");
            let mut token = [0_u8; 64];
            let _ = read_exact_with_timeout(&stream1, &mut token, Duration::from_secs(2));
            let mut prefix = [0_u8; 4];
            let _ = read_exact_with_timeout(&stream1, &mut prefix, Duration::from_secs(2));
            let len = u32::from_be_bytes(prefix) as usize;
            let mut payload = vec![0_u8; len];
            let _ = read_exact_with_timeout(&stream1, &mut payload, Duration::from_secs(2));

            // Send SubscriptionReady
            let ready = ServerResponse::success(
                "sub-req-1",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-backoff-1".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&ready).unwrap(),
                Duration::from_secs(2),
            );

            // Send Snapshot (seq 0)
            let snapshot1 = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-backoff-1", 0, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&snapshot1).unwrap(),
                Duration::from_secs(2),
            );

            // Wait for client to receive initial snapshot before forcefully disconnecting
            while !disconnect_flag_clone.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(10));
            }
            drop(stream1);

            // Reconnect loop: accept next connection and respond with new stream snapshot
            loop {
                let stream = match server.accept_with_timeout(Duration::from_millis(100)) {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                    Err(_) => break,
                };
                let mut tok = [0_u8; 64];
                if read_exact_with_timeout(&stream, &mut tok, Duration::from_secs(2)).is_err() {
                    continue;
                }
                let mut pfx = [0_u8; 4];
                if read_exact_with_timeout(&stream, &mut pfx, Duration::from_secs(2)).is_err() {
                    continue;
                }
                let l = u32::from_be_bytes(pfx) as usize;
                let mut pay = vec![0_u8; l];
                let mut frame = Vec::with_capacity(l + 4);
                frame.extend_from_slice(&pfx);
                if read_exact_with_timeout(&stream, &mut pay, Duration::from_secs(2)).is_err() {
                    continue;
                }
                frame.extend_from_slice(&pay);
                if let Ok(req) = kesharon_protocol::decode_client_request_frame(&frame) {
                    match req.method() {
                        kesharon_protocol::RequestMethod::SubscribeEvents => {
                            let ready2 = ServerResponse::success(
                                req.request_id(),
                                ResponsePayload::SubscriptionReady {
                                    stream_id: "stream-backoff-2".into(),
                                },
                            );
                            let _ = write_all_with_timeout(
                                &stream,
                                &encode_frame(&ready2).unwrap(),
                                Duration::from_secs(2),
                            );
                            let snapshot2 = StreamMessage::Snapshot {
                                snapshot: WorkspaceSnapshot::new(
                                    "stream-backoff-2",
                                    10,
                                    None,
                                    vec![],
                                ),
                            };
                            let _ = write_all_with_timeout(
                                &stream,
                                &encode_frame(&snapshot2).unwrap(),
                                Duration::from_secs(2),
                            );
                            thread::sleep(Duration::from_millis(200));
                            break;
                        }
                        kesharon_protocol::RequestMethod::Health => {
                            let busy_resp = ServerResponse::failure(
                                req.request_id(),
                                kesharon_protocol::ErrorCode::ServerBusy,
                                "daemon reconnecting",
                            );
                            let _ = write_all_with_timeout(
                                &stream,
                                &encode_frame(&busy_resp).unwrap(),
                                Duration::from_secs(2),
                            );
                        }
                        _ => {}
                    }
                }
            }
        });

        let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // 1. Initial snapshot received
        let msg1 = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("first snapshot");
        disconnect_flag.store(true, Ordering::Release);
        match msg1 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot },
            } => {
                assert_eq!(snapshot.stream_id(), "stream-backoff-1");
                assert_eq!(snapshot.last_sequence(), 0);
            }
            other => panic!("expected first snapshot, got {other:?}"),
        }

        // 2. Disconnect triggers Reconnecting update
        let msg2 = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("reconnecting update");
        assert!(
            matches!(msg2, DaemonStreamUpdate::Reconnecting),
            "expected Reconnecting update upon disconnect, got {msg2:?}"
        );

        // 3. Reconnect succeeds and sends fresh authoritative snapshot
        let msg3 = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("second snapshot");
        match msg3 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot },
            } => {
                assert_eq!(snapshot.stream_id(), "stream-backoff-2");
                assert_eq!(snapshot.last_sequence(), 10);
            }
            other => panic!("expected authoritative snapshot after reconnect, got {other:?}"),
        }

        server_thread.join().expect("server thread finished");
    });
}

// 4. Sequence gap (e.g. synthetic jump from sequence 1 to 5) triggers immediate reconnect and authoritative snapshot refresh
#[test]
fn stream_worker_detects_sequence_gap_and_recovers_via_snapshot() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");

        let server_thread = thread::spawn(move || {
            // First connection: sends snapshot 0, event 1, then sequence gap (event 5)
            let stream1 = server.accept().expect("client connects");
            let mut token = [0_u8; 64];
            let _ = read_exact_with_timeout(&stream1, &mut token, Duration::from_secs(2));
            let mut prefix = [0_u8; 4];
            let _ = read_exact_with_timeout(&stream1, &mut prefix, Duration::from_secs(2));
            let len = u32::from_be_bytes(prefix) as usize;
            let mut payload = vec![0_u8; len];
            let _ = read_exact_with_timeout(&stream1, &mut payload, Duration::from_secs(2));

            // SubscriptionReady
            let ready = ServerResponse::success(
                "sub-gap",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-gap-1".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&ready).unwrap(),
                Duration::from_secs(2),
            );

            // Snapshot (seq 0)
            let snapshot1 = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-gap-1", 0, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&snapshot1).unwrap(),
                Duration::from_secs(2),
            );

            // Event 1 (valid)
            let event1 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-gap-1",
                    1,
                    DaemonEventPayload::OperationStarted {
                        kind: OperationKind::OpenProject,
                        request_id: "r1".into(),
                    },
                )
                .expect("valid event 1"),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&event1).unwrap(),
                Duration::from_secs(2),
            );

            // Event 5 (SYNTHETIC GAP: expected sequence was 2!)
            let event5 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-gap-1",
                    5,
                    DaemonEventPayload::OperationStarted {
                        kind: OperationKind::OpenProject,
                        request_id: "r5".into(),
                    },
                )
                .expect("valid event 5"),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&event5).unwrap(),
                Duration::from_secs(2),
            );

            // Second connection after reconnect: client reconnects
            let stream2 = server.accept().expect("client reconnects after gap");
            let _ = read_exact_with_timeout(&stream2, &mut token, Duration::from_secs(2));
            let _ = read_exact_with_timeout(&stream2, &mut prefix, Duration::from_secs(2));
            let len2 = u32::from_be_bytes(prefix) as usize;
            let mut payload2 = vec![0_u8; len2];
            let _ = read_exact_with_timeout(&stream2, &mut payload2, Duration::from_secs(2));

            let ready2 = ServerResponse::success(
                "sub-gap-reconnect",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-gap-1".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&ready2).unwrap(),
                Duration::from_secs(2),
            );

            // Fresh Authoritative Snapshot covering up to sequence 5
            let fresh_snapshot = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-gap-1", 5, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&fresh_snapshot).unwrap(),
                Duration::from_secs(2),
            );

            thread::sleep(Duration::from_millis(200));
        });

        let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // 1. Initial snapshot (seq 0)
        let m1 = rx.recv_timeout(Duration::from_secs(3)).expect("m1");
        assert!(matches!(
            m1,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { .. }
            }
        ));

        // 2. Event 1
        let m2 = rx.recv_timeout(Duration::from_secs(3)).expect("m2");
        match m2 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Event { event },
            } => {
                assert_eq!(event.sequence(), 1);
            }
            other => panic!("expected event 1, got {other:?}"),
        }

        // 3. Gap detected -> Reconnecting update
        let m3 = rx.recv_timeout(Duration::from_secs(3)).expect("m3");
        assert!(
            matches!(m3, DaemonStreamUpdate::Reconnecting),
            "expected Reconnecting update upon gap detection, got {m3:?}"
        );

        // 4. Fresh Snapshot covering sequence 5
        let m4 = rx.recv_timeout(Duration::from_secs(3)).expect("m4");
        match m4 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot },
            } => {
                assert_eq!(snapshot.last_sequence(), 5);
            }
            other => panic!("expected fresh snapshot covering sequence 5, got {other:?}"),
        }

        server_thread.join().expect("server thread finished");
    });
}

// 5. Stream ID change triggers immediate reconnect and authoritative snapshot refresh
#[test]
fn stream_worker_detects_stream_id_change_and_recovers_via_snapshot() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");

        let server_thread = thread::spawn(move || {
            let stream1 = server.accept().expect("client connects");
            let mut token = [0_u8; 64];
            let _ = read_exact_with_timeout(&stream1, &mut token, Duration::from_secs(2));
            let mut prefix = [0_u8; 4];
            let _ = read_exact_with_timeout(&stream1, &mut prefix, Duration::from_secs(2));
            let len = u32::from_be_bytes(prefix) as usize;
            let mut payload = vec![0_u8; len];
            let _ = read_exact_with_timeout(&stream1, &mut payload, Duration::from_secs(2));

            // SubscriptionReady stream-alpha
            let ready = ServerResponse::success(
                "sub-id-test",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-alpha".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&ready).unwrap(),
                Duration::from_secs(2),
            );

            // Snapshot stream-alpha (seq 0)
            let snapshot1 = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-alpha", 0, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&snapshot1).unwrap(),
                Duration::from_secs(2),
            );

            // Send event from DIFFERENT stream_id ("stream-beta")
            let event_wrong_stream = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-beta",
                    1,
                    DaemonEventPayload::OperationStarted {
                        kind: OperationKind::OpenProject,
                        request_id: "r-wrong".into(),
                    },
                )
                .expect("valid event wrong stream"),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&event_wrong_stream).unwrap(),
                Duration::from_secs(2),
            );

            // Client reconnects
            let stream2 = server.accept().expect("client reconnects");
            let _ = read_exact_with_timeout(&stream2, &mut token, Duration::from_secs(2));
            let _ = read_exact_with_timeout(&stream2, &mut prefix, Duration::from_secs(2));
            let len2 = u32::from_be_bytes(prefix) as usize;
            let mut payload2 = vec![0_u8; len2];
            let _ = read_exact_with_timeout(&stream2, &mut payload2, Duration::from_secs(2));

            let ready2 = ServerResponse::success(
                "sub-id-reconnect",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-beta".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&ready2).unwrap(),
                Duration::from_secs(2),
            );

            let fresh_snapshot = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-beta", 1, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&fresh_snapshot).unwrap(),
                Duration::from_secs(2),
            );

            thread::sleep(Duration::from_millis(200));
        });

        let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // 1. Initial snapshot stream-alpha
        let m1 = rx.recv_timeout(Duration::from_secs(3)).expect("m1");
        match m1 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot },
            } => {
                assert_eq!(snapshot.stream_id(), "stream-alpha");
            }
            other => panic!("expected stream-alpha snapshot, got {other:?}"),
        }

        // 2. Stream mismatch detected -> Reconnecting update
        let m2 = rx.recv_timeout(Duration::from_secs(3)).expect("m2");
        assert!(
            matches!(m2, DaemonStreamUpdate::Reconnecting),
            "expected Reconnecting update upon stream id change, got {m2:?}"
        );

        // 3. Reconnected with stream-beta snapshot
        let m3 = rx.recv_timeout(Duration::from_secs(3)).expect("m3");
        match m3 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot },
            } => {
                assert_eq!(snapshot.stream_id(), "stream-beta");
                assert_eq!(snapshot.last_sequence(), 1);
            }
            other => panic!("expected fresh snapshot for stream-beta, got {other:?}"),
        }

        server_thread.join().expect("server thread finished");
    });
}

// 6. Daemon restart recovery: restarting the daemon process triggers client reconnect with backoff, receiving the fresh snapshot
#[test]
fn stream_worker_recovers_after_daemon_restart() {
    // Daemon 1
    let endpoint1 = unique_endpoint();
    let server1 = LocalServer::bind(&endpoint1).expect("endpoint1 available");
    let daemon1_active = Arc::new(AtomicBool::new(true));
    let daemon1_active_clone = Arc::clone(&daemon1_active);

    let server_thread1 = thread::spawn(move || {
        let stream1 = server1.accept().expect("client connects to daemon 1");
        let mut token = [0_u8; 64];
        let _ = read_exact_with_timeout(&stream1, &mut token, Duration::from_secs(2));
        let mut prefix = [0_u8; 4];
        let _ = read_exact_with_timeout(&stream1, &mut prefix, Duration::from_secs(2));
        let len = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; len];
        let _ = read_exact_with_timeout(&stream1, &mut payload, Duration::from_secs(2));

        let ready = ServerResponse::success(
            "sub-d1",
            ResponsePayload::SubscriptionReady {
                stream_id: "stream-d1".into(),
            },
        );
        let _ = write_all_with_timeout(
            &stream1,
            &encode_frame(&ready).unwrap(),
            Duration::from_secs(2),
        );

        let snapshot1 = StreamMessage::Snapshot {
            snapshot: WorkspaceSnapshot::new("stream-d1", 0, None, vec![]),
        };
        let _ = write_all_with_timeout(
            &stream1,
            &encode_frame(&snapshot1).unwrap(),
            Duration::from_secs(2),
        );

        while daemon1_active_clone.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(20));
        }

        drop(stream1);
    });

    let client1 = DaemonClient::new_with_timeout(endpoint1, TOKEN.into(), Duration::from_secs(3))
        .expect("client1 valid");
    let daemon_state = DaemonState::new_with_client(client1);

    let (channel, rx) = create_test_channel();
    execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

    // Initial snapshot from Daemon 1
    let msg1 = rx.recv_timeout(Duration::from_secs(3)).expect("msg1");
    match msg1 {
        DaemonStreamUpdate::Message {
            message: StreamMessage::Snapshot { snapshot },
        } => {
            assert_eq!(snapshot.stream_id(), "stream-d1");
            assert_eq!(snapshot.last_sequence(), 0);
        }
        other => panic!("expected stream-d1 snapshot, got {other:?}"),
    }

    // Terminate Daemon 1 (simulating crash)
    daemon1_active.store(false, Ordering::Release);
    server_thread1.join().expect("daemon 1 thread joins");
    {
        let mut client_lock = daemon_state.client.write().expect("lock client");
        client_lock.take();
    }

    // Expect Reconnecting update
    let msg2 = rx.recv_timeout(Duration::from_secs(3)).expect("msg2");
    assert!(
        matches!(msg2, DaemonStreamUpdate::Reconnecting),
        "expected Reconnecting update upon daemon termination, got {msg2:?}"
    );

    // Daemon 2 starts on new endpoint
    let endpoint2 = unique_endpoint();
    let server2 = LocalServer::bind(&endpoint2).expect("endpoint2 available");
    let daemon2 = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime2 = ServerRuntime::new(daemon2);

    let server_endpoint2 = endpoint2.clone();
    let _server_thread2 = thread::spawn(move || {
        let _ = runtime2.run(&server2);
    });

    // Update DaemonState with client 2 (simulating supervisor restarting daemon)
    let client2 =
        DaemonClient::new_with_timeout(server_endpoint2, TOKEN.into(), Duration::from_secs(3))
            .expect("client2 valid");
    {
        let mut client_lock = daemon_state.client.write().expect("lock client");
        *client_lock = Some(client2.clone());
    }

    // Stream worker should automatically reconnect to Daemon 2 and emit fresh snapshot
    let msg3 = rx.recv_timeout(Duration::from_secs(3)).expect("msg3");
    match msg3 {
        DaemonStreamUpdate::Message {
            message: StreamMessage::Snapshot { snapshot },
        } => {
            assert_eq!(snapshot.last_sequence(), 0);
        }
        other => panic!("expected fresh snapshot from restarted daemon, got {other:?}"),
    }

    // Verify live events on Daemon 2 work through the recovered stream worker
    let repo_dir = create_temp_git_repository("restart-recovery");
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    let _ = tauri::async_runtime::block_on(execute_open_project(
        &daemon_state,
        repo_path,
        Some("req-after-restart".into()),
        Some("idemp-after-restart".into()),
    ))
    .expect("open_project succeeds on restarted daemon");

    let msg4 = rx.recv_timeout(Duration::from_secs(3)).expect("msg4");
    assert!(matches!(
        msg4,
        DaemonStreamUpdate::Message {
            message: StreamMessage::Event { .. }
        }
    ));

    let _ = std::fs::remove_dir_all(&repo_dir);
}

// 7. Subscription supersedence: invoking subscribe_daemon_events again terminates the prior stream worker cleanly
#[test]
fn stream_subscription_supersedence_terminates_prior_worker() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");
        let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
        let runtime = ServerRuntime::new(daemon);

        let server_endpoint = endpoint.clone();
        let _server_thread = thread::spawn(move || {
            let _ = runtime.run(&server);
        });

        let client =
            DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(3))
                .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        // 1. Start subscription 1 on channel 1
        let (channel1, rx1) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel1).expect("sub 1 succeeds");

        let sub1_snapshot = rx1
            .recv_timeout(Duration::from_secs(3))
            .expect("sub1 snapshot");
        assert!(matches!(
            sub1_snapshot,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { .. }
            }
        ));

        // 2. Start subscription 2 on channel 2 (supersedes subscription 1)
        let (channel2, rx2) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel2).expect("sub 2 succeeds");

        let sub2_snapshot = rx2
            .recv_timeout(Duration::from_secs(3))
            .expect("sub2 snapshot");
        assert!(matches!(
            sub2_snapshot,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { .. }
            }
        ));

        // 3. Trigger live operation
        let repo_dir = create_temp_git_repository("supersedence-test");
        let repo_path = repo_dir.to_str().expect("valid path").to_owned();

        let _ = execute_open_project(
            &daemon_state,
            repo_path,
            Some("req-supersede".into()),
            Some("idemp-supersede".into()),
        )
        .await
        .expect("open succeeds");

        // Subscription 2 MUST receive the events
        let sub2_event1 = rx2
            .recv_timeout(Duration::from_secs(3))
            .expect("sub2 event1");
        assert!(matches!(
            sub2_event1,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Event { .. }
            }
        ));

        // Subscription 1 MUST NOT receive any further events because its worker terminated cleanly
        // rx1 should either receive Reconnecting (if closed before worker exited) or timeout / disconnect
        thread::sleep(Duration::from_millis(150));
        let remaining_messages: Vec<_> = rx1.try_iter().collect();
        for msg in remaining_messages {
            // Must not contain the new Operation event
            if let DaemonStreamUpdate::Message {
                message: StreamMessage::Event { event },
            } = msg
            {
                panic!("Old subscription received event: {event:?}");
            }
        }

        let _ = std::fs::remove_dir_all(&repo_dir);
    });
}
