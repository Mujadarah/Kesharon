#![allow(
    clippy::all,
    clippy::pedantic,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::collapsible_match,
    clippy::collapsible_if,
    clippy::items_after_statements,
    clippy::duration_suboptimal_units,
    clippy::needless_continue,
    clippy::match_wildcard_for_single_variants
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_desktop_host::commands::{
    execute_cancel_request, execute_open_project, execute_subscribe_daemon_events,
};
use kesharon_desktop_host::{
    DaemonClient, DaemonState, DaemonStreamUpdate, INITIAL_BACKOFF, MAX_BACKOFF, next_backoff,
};
use kesharon_ipc::{
    LocalEndpoint, LocalServer, LocalStream, read_exact_with_timeout, write_all_with_timeout,
};
use kesharon_protocol::{
    DaemonEvent, DaemonEventPayload, LaunchToken, OperationKind, ProjectSnapshot, ResponsePayload,
    ServerResponse, StreamMessage, WorkspaceSnapshot, decode_stream_message_frame, encode_frame,
};
use tauri::ipc::Channel;

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(9500);

fn unique_endpoint() -> LocalEndpoint {
    let endpoint_id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    let value = format!("kesharon-chal2-test-{}-{}", std::process::id(), endpoint_id);

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "k-chal2-{:x}-{endpoint_id:x}.sock",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

fn create_temp_git_repository(name: &str) -> std::path::PathBuf {
    let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let repo_dir = std::env::temp_dir().join(format!(
        "kesharon-chal2-repo-{}-{}-{}",
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
    let trimmed = json.trim();
    if trimmed.contains("\"type\":\"reconnecting\"") {
        return DaemonStreamUpdate::Reconnecting;
    }
    if trimmed.contains("\"type\":\"unavailable\"") {
        return DaemonStreamUpdate::Unavailable {
            message: "unavailable".into(),
        };
    }
    if let Some(msg_idx) = trimmed.find("\"message\":") {
        let msg_start = msg_idx + 10;
        if msg_start < trimmed.len() && trimmed.ends_with('}') {
            let msg_json = trimmed[msg_start..trimmed.len() - 1].trim();
            let bytes = msg_json.as_bytes();
            let len = bytes.len() as u32;
            let mut frame = Vec::with_capacity(bytes.len() + 4);
            frame.extend_from_slice(&len.to_be_bytes());
            frame.extend_from_slice(bytes);
            if let Ok(message) = decode_stream_message_frame(&frame) {
                return DaemonStreamUpdate::Message { message };
            }
        }
    }
    panic!("unrecognized DaemonStreamUpdate JSON: {json}");
}

fn recv_next_snapshot(
    rx: &std::sync::mpsc::Receiver<DaemonStreamUpdate>,
    timeout: Duration,
) -> WorkspaceSnapshot {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Ok(update) = rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
            if let DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot },
            } = update
            {
                return snapshot;
            }
        }
    }
    panic!("timed out waiting for WorkspaceSnapshot");
}

// Challenge 1: Single & Multi-Cycle Daemon Restart Recovery
#[test]
fn empirical_daemon_restart_recovery_with_stream_resubscription() {
    let endpoint1 = unique_endpoint();
    let server1 = LocalServer::bind(&endpoint1).expect("endpoint1 available");
    let active_flag1 = Arc::new(AtomicBool::new(true));
    let active_flag1_clone = Arc::clone(&active_flag1);

    let server_thread1 = thread::spawn(move || {
        let stream1 = server1.accept().expect("client connects daemon 1");
        let mut tok = [0_u8; 64];
        let _ = read_exact_with_timeout(&stream1, &mut tok, Duration::from_secs(2));
        let mut pfx = [0_u8; 4];
        let _ = read_exact_with_timeout(&stream1, &mut pfx, Duration::from_secs(2));
        let len = u32::from_be_bytes(pfx) as usize;
        let mut pay = vec![0_u8; len];
        let _ = read_exact_with_timeout(&stream1, &mut pay, Duration::from_secs(2));

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

        while active_flag1_clone.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(20));
        }

        drop(stream1);
    });

    let client1 = DaemonClient::new_with_timeout(endpoint1, TOKEN.into(), Duration::from_secs(3))
        .expect("client1 valid");
    let daemon_state = DaemonState::new_with_client(client1);

    let (channel, rx) = create_test_channel();
    execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

    // 1. Initial snapshot from Daemon 1
    let snap1 = recv_next_snapshot(&rx, Duration::from_secs(3));
    assert_eq!(snap1.stream_id(), "stream-d1");
    assert_eq!(snap1.last_sequence(), 0);

    // 2. Terminate Daemon 1 (simulating crash / restart)
    active_flag1.store(false, Ordering::Release);
    server_thread1.join().expect("daemon 1 thread joins");
    {
        let mut client_lock = daemon_state.client.write().expect("lock client");
        client_lock.take();
    }

    // Expect Reconnecting update
    let update = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("reconnecting update");
    assert!(
        matches!(update, DaemonStreamUpdate::Reconnecting),
        "expected Reconnecting update upon daemon crash, got {update:?}"
    );

    // 3. Start Daemon 2 on fresh endpoint
    let endpoint2 = unique_endpoint();
    let server2 = LocalServer::bind(&endpoint2).expect("endpoint2 available");
    let daemon2 = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime2 = ServerRuntime::new(daemon2);

    let server_endpoint2 = endpoint2.clone();
    let _server_thread2 = thread::spawn(move || {
        let _ = runtime2.run(&server2);
    });

    // 4. Update DaemonState with client 2 (simulating supervisor restarting daemon)
    let client2 =
        DaemonClient::new_with_timeout(server_endpoint2, TOKEN.into(), Duration::from_secs(3))
            .expect("client2 valid");
    {
        let mut client_lock = daemon_state.client.write().expect("lock client");
        *client_lock = Some(client2);
    }

    // 5. Stream worker must auto-reconnect to Daemon 2 and emit fresh snapshot
    let snap2 = recv_next_snapshot(&rx, Duration::from_secs(4));
    assert_eq!(snap2.last_sequence(), 0);

    // 6. Verify live operations work on the restarted daemon
    let repo_dir = create_temp_git_repository("restart-recov");
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    let open_res = tauri::async_runtime::block_on(execute_open_project(
        &daemon_state,
        repo_path,
        Some("req-after-restart".into()),
        Some("idemp-after-restart".into()),
    ))
    .expect("open succeeds on restarted daemon");

    assert!(matches!(
        open_res.result(),
        Some(ResponsePayload::ProjectOpened { .. })
    ));

    let _ = std::fs::remove_dir_all(&repo_dir);
}

// Challenge 2: Rapid Burst Subscription Supersedence (25 rapid subscriptions)
#[test]
fn empirical_rapid_burst_subscription_supersedence() {
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

        const BURST_COUNT: usize = 25;
        let mut receivers = Vec::with_capacity(BURST_COUNT);

        // Rapidly subscribe 25 times
        for _ in 0..BURST_COUNT {
            let (channel, rx) = create_test_channel();
            execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");
            receivers.push(rx);
        }

        // The final receiver (index 24) is the active one.
        let active_rx = receivers.pop().expect("has active receiver");

        // Wait briefly for previous workers to observe cancellation and exit
        thread::sleep(Duration::from_millis(200));

        // Initial snapshot on active receiver
        let initial_snapshot = active_rx
            .recv_timeout(Duration::from_secs(4))
            .expect("active channel receives snapshot");
        assert!(matches!(
            initial_snapshot,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { .. }
            }
        ));

        // Drain any pre-cancellation initial snapshot messages from old receivers
        for rx in &receivers {
            let _ = rx.try_iter().collect::<Vec<_>>();
        }

        // Trigger a live operation
        let repo_dir = create_temp_git_repository("supersedence-burst");
        let repo_path = repo_dir.to_str().expect("valid path").to_owned();

        let _ = execute_open_project(
            &daemon_state,
            repo_path,
            Some("req-burst".into()),
            Some("idemp-burst".into()),
        )
        .await
        .expect("open succeeds");

        // The active channel MUST receive the live events
        let ev1 = active_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("active channel receives event 1");
        assert!(matches!(
            ev1,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Event { .. }
            }
        ));

        let ev2 = active_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("active channel receives event 2");
        assert!(matches!(
            ev2,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Event { .. }
            }
        ));

        // Allow any lingering messages in flight to be delivered
        thread::sleep(Duration::from_millis(200));

        // Verify NONE of the 24 superseded receivers received the live events
        for (i, rx) in receivers.iter().enumerate() {
            let messages: Vec<_> = rx.try_iter().collect();
            for msg in messages {
                if let DaemonStreamUpdate::Message {
                    message: StreamMessage::Event { event },
                } = msg
                {
                    panic!(
                        "Superseded channel {i} received unexpected live event: seq={}, payload={:?}",
                        event.sequence(),
                        event.payload()
                    );
                }
            }
        }

        let _ = std::fs::remove_dir_all(&repo_dir);
    });
}

// Challenge 3: Concurrent Multi-Thread Subscription Race
#[test]
fn empirical_concurrent_multi_thread_subscription_supersedence() {
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
        let daemon_state = Arc::new(DaemonState::new_with_client(client));

        const THREAD_COUNT: usize = 10;
        let barrier = Arc::new(std::sync::Barrier::new(THREAD_COUNT));
        let mut handles = Vec::with_capacity(THREAD_COUNT);
        let mut receivers = Vec::with_capacity(THREAD_COUNT);

        for _ in 0..THREAD_COUNT {
            let state_clone = Arc::clone(&daemon_state);
            let barrier_clone = Arc::clone(&barrier);
            let (channel, rx) = create_test_channel();
            receivers.push(rx);

            let handle = thread::spawn(move || {
                barrier_clone.wait();
                execute_subscribe_daemon_events(&state_clone, channel).expect("subscribe succeeds");
            });
            handles.push(handle);
        }

        for h in handles {
            h.join().expect("thread finished");
        }

        // Settle time for race to resolve and workers to establish/cancel
        thread::sleep(Duration::from_millis(300));

        // Drain any initial snapshots
        for rx in &receivers {
            let _ = rx.try_iter().collect::<Vec<_>>();
        }

        // Trigger live operation
        let repo_dir = create_temp_git_repository("concurrent-race");
        let repo_path = repo_dir.to_str().expect("valid path").to_owned();

        let _ = execute_open_project(
            &daemon_state,
            repo_path,
            Some("req-concurrent".into()),
            Some("idemp-concurrent".into()),
        )
        .await
        .expect("open succeeds");

        thread::sleep(Duration::from_millis(300));

        // Count how many channels received live events
        let mut channels_with_events = 0;
        for rx in &receivers {
            let msgs: Vec<_> = rx.try_iter().collect();
            let has_event = msgs.iter().any(|m| {
                matches!(
                    m,
                    DaemonStreamUpdate::Message {
                        message: StreamMessage::Event { .. }
                    }
                )
            });
            if has_event {
                channels_with_events += 1;
            }
        }

        // Exactly 1 winning channel must receive live events
        assert_eq!(
            channels_with_events, 1,
            "Expected exactly 1 active subscription to receive events after concurrent subscribe race, found {channels_with_events}"
        );

        let _ = std::fs::remove_dir_all(&repo_dir);
    });
}

// Challenge 4: Stream Worker Exits Promptly on Frontend Channel Disconnect
#[test]
fn empirical_subscription_cleanup_on_channel_close() {
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

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // Wait for snapshot
        let _ = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("snapshot received");

        // Drop the receiver (simulating webview reload or unmount)
        drop(rx);

        // Trigger live operation
        let repo_dir = create_temp_git_repository("channel-close-test");
        let repo_path = repo_dir.to_str().expect("valid path").to_owned();

        let _ = execute_open_project(
            &daemon_state,
            repo_path,
            Some("req-close".into()),
            Some("idemp-close".into()),
        )
        .await
        .expect("open succeeds");

        // Worker should detect send error and terminate without panic or hanging
        thread::sleep(Duration::from_millis(200));

        let _ = std::fs::remove_dir_all(&repo_dir);
    });
}

// Challenge 5: Repeated Stream ID Flips (stream-1 -> stream-2 -> stream-3 -> stream-4)
#[test]
fn empirical_repeated_stream_id_flips() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");

        let server_thread = thread::spawn(move || {
            // Helper to handle subscription handshake
            let handle_sub = |stream: &LocalStream, stream_id: &str, last_seq: u64| {
                let mut tok = [0_u8; 64];
                let _ = read_exact_with_timeout(stream, &mut tok, Duration::from_secs(2));
                let mut pfx = [0_u8; 4];
                let _ = read_exact_with_timeout(stream, &mut pfx, Duration::from_secs(2));
                let len = u32::from_be_bytes(pfx) as usize;
                let mut pay = vec![0_u8; len];
                let _ = read_exact_with_timeout(stream, &mut pay, Duration::from_secs(2));

                let ready = ServerResponse::success(
                    "sub-flip",
                    ResponsePayload::SubscriptionReady {
                        stream_id: stream_id.into(),
                    },
                );
                let _ = write_all_with_timeout(
                    stream,
                    &encode_frame(&ready).unwrap(),
                    Duration::from_secs(2),
                );

                let snapshot = StreamMessage::Snapshot {
                    snapshot: WorkspaceSnapshot::new(stream_id, last_seq, None, vec![]),
                };
                let _ = write_all_with_timeout(
                    stream,
                    &encode_frame(&snapshot).unwrap(),
                    Duration::from_secs(2),
                );
            };

            // Stream 1
            let stream1 = server.accept().expect("client connects stream 1");
            handle_sub(&stream1, "stream-1", 0);

            // Inject mismatch: send event with stream-2
            let flip_event1 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-2",
                    1,
                    DaemonEventPayload::OperationStarted {
                        request_id: "r1".into(),
                        kind: OperationKind::OpenProject,
                    },
                )
                .unwrap(),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&flip_event1).unwrap(),
                Duration::from_secs(2),
            );

            // Stream 2
            let stream2 = server
                .accept()
                .expect("client connects after stream flip to stream 2");
            handle_sub(&stream2, "stream-2", 1);

            // Inject mismatch: send event with stream-3
            let flip_event2 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-3",
                    2,
                    DaemonEventPayload::OperationStarted {
                        request_id: "r2".into(),
                        kind: OperationKind::OpenProject,
                    },
                )
                .unwrap(),
            };
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&flip_event2).unwrap(),
                Duration::from_secs(2),
            );

            // Stream 3
            let stream3 = server
                .accept()
                .expect("client connects after stream flip to stream 3");
            handle_sub(&stream3, "stream-3", 2);

            // Inject mismatch: send event with stream-4
            let flip_event3 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-4",
                    3,
                    DaemonEventPayload::OperationStarted {
                        request_id: "r3".into(),
                        kind: OperationKind::OpenProject,
                    },
                )
                .unwrap(),
            };
            let _ = write_all_with_timeout(
                &stream3,
                &encode_frame(&flip_event3).unwrap(),
                Duration::from_secs(2),
            );

            // Stream 4 (final valid stream)
            let stream4 = server
                .accept()
                .expect("client connects after stream flip to stream 4");
            handle_sub(&stream4, "stream-4", 10);

            // Send valid event matching stream 4 and sequence 11
            let valid_event = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-4",
                    11,
                    DaemonEventPayload::ProjectOpened {
                        request_id: "r4".into(),
                        project: ProjectSnapshot {
                            id: "proj-4".into(),
                            canonical_root: "d:/test/path".into(),
                            display_name: "test-repo".into(),
                            trusted: true,
                        },
                    },
                )
                .unwrap(),
            };
            let _ = write_all_with_timeout(
                &stream4,
                &encode_frame(&valid_event).unwrap(),
                Duration::from_secs(2),
            );

            thread::sleep(Duration::from_millis(200));
        });

        let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // 1. Initial snapshot stream-1
        let m1 = rx.recv_timeout(Duration::from_secs(3)).expect("m1");
        assert!(matches!(
            m1,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.stream_id() == "stream-1"
        ));

        // 2. Reconnecting update -> Snapshot stream-2
        let m2 = rx.recv_timeout(Duration::from_secs(3)).expect("m2");
        assert!(matches!(m2, DaemonStreamUpdate::Reconnecting));

        let m3 = rx.recv_timeout(Duration::from_secs(3)).expect("m3");
        assert!(matches!(
            m3,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.stream_id() == "stream-2" && snapshot.last_sequence() == 1
        ));

        // 3. Reconnecting update -> Snapshot stream-3
        let m4 = rx.recv_timeout(Duration::from_secs(3)).expect("m4");
        assert!(matches!(m4, DaemonStreamUpdate::Reconnecting));

        let m5 = rx.recv_timeout(Duration::from_secs(3)).expect("m5");
        assert!(matches!(
            m5,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.stream_id() == "stream-3" && snapshot.last_sequence() == 2
        ));

        // 4. Reconnecting update -> Snapshot stream-4
        let m6 = rx.recv_timeout(Duration::from_secs(3)).expect("m6");
        assert!(matches!(m6, DaemonStreamUpdate::Reconnecting));

        let m7 = rx.recv_timeout(Duration::from_secs(3)).expect("m7");
        assert!(matches!(
            m7,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.stream_id() == "stream-4" && snapshot.last_sequence() == 10
        ));

        // 5. Valid live event on stream-4 (seq 11)
        let m8 = rx.recv_timeout(Duration::from_secs(3)).expect("m8");
        match m8 {
            DaemonStreamUpdate::Message {
                message: StreamMessage::Event { event },
            } => {
                assert_eq!(event.stream_id(), "stream-4");
                assert_eq!(event.sequence(), 11);
            }
            other => panic!("expected valid event on stream-4, got {other:?}"),
        }

        server_thread.join().expect("server thread finished");
    });
}

// Challenge 6: Sequence Anomalies: Duplicate Sequence and Backward Sequence Jump
#[test]
fn empirical_sequence_backward_and_duplicate_anomalies() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");

        let server_thread = thread::spawn(move || {
            let stream1 = server.accept().expect("client connects");
            let mut tok = [0_u8; 64];
            let _ = read_exact_with_timeout(&stream1, &mut tok, Duration::from_secs(2));
            let mut pfx = [0_u8; 4];
            let _ = read_exact_with_timeout(&stream1, &mut pfx, Duration::from_secs(2));
            let len = u32::from_be_bytes(pfx) as usize;
            let mut pay = vec![0_u8; len];
            let _ = read_exact_with_timeout(&stream1, &mut pay, Duration::from_secs(2));

            let ready = ServerResponse::success(
                "sub-seq-anom",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-anom".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&ready).unwrap(),
                Duration::from_secs(2),
            );

            // Snapshot (seq 10)
            let snapshot1 = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-anom", 10, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&snapshot1).unwrap(),
                Duration::from_secs(2),
            );

            // Event 11 (valid)
            let event11 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-anom",
                    11,
                    DaemonEventPayload::OperationStarted {
                        request_id: "r11".into(),
                        kind: OperationKind::OpenProject,
                    },
                )
                .unwrap(),
            };
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&event11).unwrap(),
                Duration::from_secs(2),
            );

            // Event 11 again! (DUPLICATE / ANOMALY: expected was 12)
            let _ = write_all_with_timeout(
                &stream1,
                &encode_frame(&event11).unwrap(),
                Duration::from_secs(2),
            );

            // Client reconnects
            let stream2 = server
                .accept()
                .expect("client reconnects after duplicate anomaly");
            let _ = read_exact_with_timeout(&stream2, &mut tok, Duration::from_secs(2));
            let _ = read_exact_with_timeout(&stream2, &mut pfx, Duration::from_secs(2));
            let len2 = u32::from_be_bytes(pfx) as usize;
            let mut pay2 = vec![0_u8; len2];
            let _ = read_exact_with_timeout(&stream2, &mut pay2, Duration::from_secs(2));

            let ready2 = ServerResponse::success(
                "sub-seq-anom2",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-anom".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&ready2).unwrap(),
                Duration::from_secs(2),
            );

            // Fresh Snapshot (seq 12)
            let snapshot2 = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-anom", 12, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&snapshot2).unwrap(),
                Duration::from_secs(2),
            );

            // Event 5 (BACKWARD SEQUENCE JUMP: expected was 13)
            let event5 = StreamMessage::Event {
                event: DaemonEvent::new(
                    "stream-anom",
                    5,
                    DaemonEventPayload::OperationStarted {
                        request_id: "r5".into(),
                        kind: OperationKind::OpenProject,
                    },
                )
                .unwrap(),
            };
            let _ = write_all_with_timeout(
                &stream2,
                &encode_frame(&event5).unwrap(),
                Duration::from_secs(2),
            );

            // Client reconnects again
            let stream3 = server
                .accept()
                .expect("client reconnects after backward jump");
            let _ = read_exact_with_timeout(&stream3, &mut tok, Duration::from_secs(2));
            let _ = read_exact_with_timeout(&stream3, &mut pfx, Duration::from_secs(2));
            let len3 = u32::from_be_bytes(pfx) as usize;
            let mut pay3 = vec![0_u8; len3];
            let _ = read_exact_with_timeout(&stream3, &mut pay3, Duration::from_secs(2));

            let ready3 = ServerResponse::success(
                "sub-seq-anom3",
                ResponsePayload::SubscriptionReady {
                    stream_id: "stream-anom".into(),
                },
            );
            let _ = write_all_with_timeout(
                &stream3,
                &encode_frame(&ready3).unwrap(),
                Duration::from_secs(2),
            );

            // Final Snapshot (seq 20)
            let snapshot3 = StreamMessage::Snapshot {
                snapshot: WorkspaceSnapshot::new("stream-anom", 20, None, vec![]),
            };
            let _ = write_all_with_timeout(
                &stream3,
                &encode_frame(&snapshot3).unwrap(),
                Duration::from_secs(2),
            );

            thread::sleep(Duration::from_millis(200));
        });

        let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // 1. Initial snapshot (seq 10)
        let m1 = rx.recv_timeout(Duration::from_secs(3)).expect("m1");
        assert!(matches!(
            m1,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.last_sequence() == 10
        ));

        // 2. Valid Event 11
        let m2 = rx.recv_timeout(Duration::from_secs(3)).expect("m2");
        assert!(matches!(
            m2,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Event { event }
            } if event.sequence() == 11
        ));

        // 3. Duplicate event 11 detected -> Reconnecting update
        let m3 = rx.recv_timeout(Duration::from_secs(3)).expect("m3");
        assert!(matches!(m3, DaemonStreamUpdate::Reconnecting));

        // 4. Reconnected Snapshot (seq 12)
        let m4 = rx.recv_timeout(Duration::from_secs(3)).expect("m4");
        assert!(matches!(
            m4,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.last_sequence() == 12
        ));

        // 5. Backward sequence detected -> Reconnecting update
        let m5 = rx.recv_timeout(Duration::from_secs(3)).expect("m5");
        assert!(matches!(m5, DaemonStreamUpdate::Reconnecting));

        // 6. Final Snapshot (seq 20)
        let m6 = rx.recv_timeout(Duration::from_secs(3)).expect("m6");
        assert!(matches!(
            m6,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { snapshot }
            } if snapshot.last_sequence() == 20
        ));

        server_thread.join().expect("server thread finished");
    });
}

// Challenge 7: Abrupt Socket Termination during Handshake Phases
#[test]
fn empirical_abrupt_socket_termination_during_handshake_phases() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        let server = LocalServer::bind(&endpoint).expect("endpoint available");

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);

        let server_thread = thread::spawn(move || {
            let read_req = |stream: &LocalStream| {
                let mut tok = [0_u8; 64];
                let _ = read_exact_with_timeout(stream, &mut tok, Duration::from_secs(1));
                let mut pfx = [0_u8; 4];
                let _ = read_exact_with_timeout(stream, &mut pfx, Duration::from_secs(1));
                let len = u32::from_be_bytes(pfx) as usize;
                let mut pay = vec![0_u8; len];
                let _ = read_exact_with_timeout(stream, &mut pay, Duration::from_secs(1));
            };

            // Phase 1: Server drops stream immediately upon accept
            if let Ok(stream) = server.accept() {
                drop(stream);
            }

            // Phase 2: Server drops stream after reading token but before sending SubscriptionReady
            if let Ok(stream) = server.accept() {
                let mut tok = [0_u8; 64];
                let _ = read_exact_with_timeout(&stream, &mut tok, Duration::from_secs(1));
                drop(stream);
            }

            // Phase 3: Server sends SubscriptionReady, then drops before snapshot length prefix
            if let Ok(stream) = server.accept() {
                read_req(&stream);
                let ready = ServerResponse::success(
                    "sub-p3",
                    ResponsePayload::SubscriptionReady {
                        stream_id: "stream-p3".into(),
                    },
                );
                let _ = write_all_with_timeout(
                    &stream,
                    &encode_frame(&ready).unwrap(),
                    Duration::from_secs(1),
                );
                drop(stream);
            }

            // Phase 4 (Final Recovery): Normal healthy stream
            if let Ok(stream) = server.accept() {
                read_req(&stream);
                let ready = ServerResponse::success(
                    "sub-p4",
                    ResponsePayload::SubscriptionReady {
                        stream_id: "stream-p4".into(),
                    },
                );
                let _ = write_all_with_timeout(
                    &stream,
                    &encode_frame(&ready).unwrap(),
                    Duration::from_secs(1),
                );

                let snapshot = StreamMessage::Snapshot {
                    snapshot: WorkspaceSnapshot::new("stream-p4", 42, None, vec![]),
                };
                let _ = write_all_with_timeout(
                    &stream,
                    &encode_frame(&snapshot).unwrap(),
                    Duration::from_secs(1),
                );

                while !stop_flag_clone.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(50));
                }
            }
        });

        // Use a 300ms I/O timeout for client so abrupt drops fail fast and test completes promptly
        let client =
            DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(300))
                .expect("client valid");
        let daemon_state = DaemonState::new_with_client(client);

        let (channel, rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        // The worker should navigate through the abrupt handshake drops and receive the snapshot from Phase 4
        let snapshot = recv_next_snapshot(&rx, Duration::from_secs(5));
        assert_eq!(snapshot.stream_id(), "stream-p4");
        assert_eq!(snapshot.last_sequence(), 42);

        stop_flag.store(true, Ordering::Release);
        server_thread.join().expect("server thread finished");
    });
}

// Challenge 8: Empirical Backoff Schedule Bounds Verification
#[test]
fn empirical_exponential_backoff_timing_bounds() {
    let mut current = INITIAL_BACKOFF;
    let mut delays = Vec::new();

    // Verify doubling formula and cap
    for _ in 0..10 {
        delays.push(current);
        current = next_backoff(current);
    }

    assert_eq!(delays[0], Duration::from_millis(50));
    assert_eq!(delays[1], Duration::from_millis(100));
    assert_eq!(delays[2], Duration::from_millis(200));
    assert_eq!(delays[3], Duration::from_millis(400));
    assert_eq!(delays[4], Duration::from_millis(800));
    assert_eq!(delays[5], Duration::from_secs(1));
    assert_eq!(delays[6], Duration::from_secs(1));
    assert_eq!(delays[7], Duration::from_secs(1));
    assert_eq!(delays[8], Duration::from_secs(1));
    assert_eq!(delays[9], Duration::from_secs(1));
    assert_eq!(MAX_BACKOFF, Duration::from_secs(1));

    // Verify empirical connection retry timing when server is not available
    let endpoint = unique_endpoint();
    let client =
        DaemonClient::new_with_timeout(endpoint.clone(), TOKEN.into(), Duration::from_secs(1))
            .expect("client valid");
    let daemon_state = DaemonState::new_with_client(client);

    let (channel, rx) = create_test_channel();
    let start_time = Instant::now();
    execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

    // Let the worker retry in backoff loop for ~450ms (attempts: 50ms + 100ms + 200ms = 350ms)
    thread::sleep(Duration::from_millis(400));

    // Now start the daemon server
    let server = LocalServer::bind(&endpoint).expect("bind server");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    // Worker should connect on its next retry and send initial snapshot
    let snap_msg = rx
        .recv_timeout(Duration::from_secs(4))
        .expect("snapshot received after delayed server startup");
    let elapsed = start_time.elapsed();

    assert!(
        matches!(
            snap_msg,
            DaemonStreamUpdate::Message {
                message: StreamMessage::Snapshot { .. }
            }
        ),
        "Expected initial snapshot once server came online"
    );

    // Elapsed time should reflect at least the initial backoff delay
    assert!(
        elapsed >= Duration::from_millis(350),
        "Elapsed time ({elapsed:?}) should be at least 350ms given the delayed startup"
    );
}

// Challenge 9: Concurrent In-Flight Mutations During Active Stream Churn
#[test]
fn empirical_concurrent_mutations_during_stream_churn() {
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
        let daemon_state = Arc::new(DaemonState::new_with_client(client));

        let (channel, _rx) = create_test_channel();
        execute_subscribe_daemon_events(&daemon_state, channel).expect("subscribe succeeds");

        let repo_dir = create_temp_git_repository("concurrent-mutation-churn");
        let repo_path = repo_dir.to_str().expect("valid path").to_owned();

        // Spawn concurrent mutations while subscription is running
        const TASK_COUNT: usize = 12;
        let mut handles = Vec::new();

        for i in 0..TASK_COUNT {
            let state_clone = Arc::clone(&daemon_state);
            let path_clone = repo_path.clone();
            let handle = tauri::async_runtime::spawn(async move {
                if i % 2 == 0 {
                    execute_open_project(
                        &state_clone,
                        path_clone,
                        Some(format!("req-churn-{i}")),
                        Some(format!("idemp-churn-{i}")),
                    )
                    .await
                } else {
                    execute_cancel_request(
                        &state_clone,
                        format!("req-churn-{}", i - 1),
                        Some(format!("req-cancel-{i}")),
                        Some(format!("idemp-cancel-{i}")),
                    )
                    .await
                }
            });
            handles.push(handle);
        }

        for (i, h) in handles.into_iter().enumerate() {
            let res = h.await.expect("task joins");
            assert!(
                res.is_ok(),
                "Task {i} failed during stream churn: {:?}",
                res.err()
            );
        }

        let _ = std::fs::remove_dir_all(&repo_dir);
    });
}
