use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kesharon_daemon::{Daemon, ServerRuntime};
use kesharon_desktop_host::commands::{
    execute_cancel_request, execute_health, execute_open_project,
};
use kesharon_desktop_host::{DaemonClient, DaemonState, HostError};
use kesharon_ipc::{LocalEndpoint, LocalServer};
use kesharon_protocol::{
    ClientRequest, ErrorCode, LaunchToken, MAX_FRAME_BYTES, ProtocolError, RequestMethod,
    ResponsePayload, ServerResponse, StreamMessage, encode_frame,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
static NEXT_ENDPOINT_ID: AtomicU64 = AtomicU64::new(1000);

fn unique_endpoint() -> LocalEndpoint {
    let endpoint_id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);

    #[cfg(windows)]
    let value = format!(
        "kesharon-stress-test-{}-{}",
        std::process::id(),
        endpoint_id
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "k-stress-{:x}-{endpoint_id:x}.sock",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

fn create_temp_git_repository() -> std::path::PathBuf {
    let id = NEXT_ENDPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let repo_dir = std::env::temp_dir().join(format!(
        "kesharon-stress-repo-{}-{}",
        std::process::id(),
        id
    ));
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("the test repo dir is creatable");
    repo_dir
}

// 1. Rapid Sequential Requests
#[test]
fn stress_rapid_sequential_requests() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(5))
            .expect("client valid");
    let daemon_state = DaemonState::new_with_client(client);

    let repo_dir = create_temp_git_repository();
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    tauri::async_runtime::block_on(async {
        for i in 0..60 {
            let req_id = format!("seq-req-{i}");
            let idemp_key = format!("idemp-key-{i}");

            if i % 3 == 0 {
                let res = execute_health(&daemon_state)
                    .await
                    .expect("health succeeds");
                assert!(matches!(res.result(), Some(ResponsePayload::Health { .. })));
            } else if i % 3 == 1 {
                let res = execute_open_project(
                    &daemon_state,
                    repo_path.clone(),
                    Some(req_id.clone()),
                    Some(idemp_key),
                )
                .await
                .expect("open_project succeeds");
                assert_eq!(res.request_id(), &req_id);
                assert!(matches!(
                    res.result(),
                    Some(ResponsePayload::ProjectOpened { .. })
                ));
            } else {
                let res = execute_cancel_request(
                    &daemon_state,
                    format!("target-{i}"),
                    Some(req_id.clone()),
                    Some(idemp_key),
                )
                .await
                .expect("cancel_request succeeds");
                assert_eq!(res.request_id(), &req_id);
                assert!(matches!(
                    res.result(),
                    Some(ResponsePayload::Cancellation { .. })
                ));
            }
        }
    });

    let _ = std::fs::remove_dir_all(&repo_dir);
}

// 2. High Concurrency Dispatch (Exceeding Server Active Connection Limit)
#[test]
fn stress_concurrent_command_dispatch() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(5))
            .expect("client valid");
    let daemon_state = Arc::new(DaemonState::new_with_client(client));

    let repo_dir = create_temp_git_repository();
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    let concurrency = 24; // > 8 MAX_ACTIVE_CONNECTIONS
    let mut handles = Vec::with_capacity(concurrency);

    for i in 0..concurrency {
        let state = Arc::clone(&daemon_state);
        let path = repo_path.clone();
        let handle = thread::spawn(move || {
            tauri::async_runtime::block_on(async move {
                let req_id = format!("conc-req-{i}");
                let idemp_key = format!("conc-idemp-{i}");
                match i % 3 {
                    0 => {
                        let res = execute_health(&state).await;
                        assert!(res.is_ok(), "health command should not crash");
                    }
                    1 => {
                        let res = execute_open_project(
                            &state,
                            path,
                            Some(req_id.clone()),
                            Some(idemp_key),
                        )
                        .await;
                        assert!(res.is_ok(), "open_project should complete gracefully");
                        let res = res.unwrap();
                        // May succeed or return ServerBusy error payload, but must not hang
                        if let Some(err) = res.error() {
                            assert_eq!(err.code(), ErrorCode::ServerBusy);
                        } else {
                            assert!(matches!(
                                res.result(),
                                Some(ResponsePayload::ProjectOpened { .. })
                            ));
                        }
                    }
                    _ => {
                        let res = execute_cancel_request(
                            &state,
                            format!("target-{i}"),
                            Some(req_id.clone()),
                            Some(idemp_key),
                        )
                        .await;
                        assert!(res.is_ok(), "cancel_request should complete gracefully");
                        let res = res.unwrap();
                        if let Some(err) = res.error() {
                            assert_eq!(err.code(), ErrorCode::ServerBusy);
                        } else {
                            assert!(matches!(
                                res.result(),
                                Some(ResponsePayload::Cancellation { .. })
                            ));
                        }
                    }
                }
            });
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("thread finished cleanly");
    }

    let _ = std::fs::remove_dir_all(&repo_dir);
}

// 3. Large Payload Handling & Oversized Frame Protection
#[test]
fn stress_oversized_frame_rejection() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        stream.read_exact(&mut token).expect("token readable");
        let mut prefix = [0_u8; 4];
        stream.read_exact(&mut prefix).expect("prefix readable");
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload).expect("payload readable");

        // Send back an oversized frame length prefix (e.g. 10MB > 8MB MAX_FRAME_BYTES)
        let oversized_len: u32 = 10_000_000;
        stream
            .write_all(&oversized_len.to_be_bytes())
            .expect("write prefix");
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(500))
        .expect("client valid");

    let req = ClientRequest::new("req-large", RequestMethod::Health, None).unwrap();
    let result = client.send_request(&req);

    assert!(result.is_err(), "should reject oversized frame");
    match result.err().unwrap() {
        HostError::Protocol(ProtocolError::FrameTooLarge { declared, maximum }) => {
            assert_eq!(declared, 10_000_000);
            assert_eq!(maximum, MAX_FRAME_BYTES);
        }
        other => panic!("expected FrameTooLarge error, got {other:?}"),
    }

    server_thread.join().expect("server thread finished");
}

// 4. Connection Drop Resilience: Immediate Close upon Accept
#[test]
fn resilience_server_immediate_disconnect() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let stream = server.accept().expect("client connects");
        // Drop stream immediately without reading or writing
        drop(stream);
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(200))
        .expect("client valid");

    let started = Instant::now();
    let result = client.health();
    let elapsed = started.elapsed();

    assert!(result.is_err(), "immediate disconnect must return Err");
    assert!(
        elapsed < Duration::from_millis(500),
        "must fail quickly within timeout"
    );

    server_thread.join().expect("server thread finished");
}

// 5. Connection Drop Resilience: Truncated Response Frame
#[test]
fn resilience_truncated_response_frame() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        let _ = stream.read_exact(&mut token);
        let mut prefix = [0_u8; 4];
        let _ = stream.read_exact(&mut prefix);
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        let _ = stream.read_exact(&mut payload);

        // Send 4-byte prefix declaring 100 bytes, but send only 10 bytes then close
        let declared: u32 = 100;
        let _ = stream.write_all(&declared.to_be_bytes());
        let _ = stream.write_all(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        drop(stream);
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(200))
        .expect("client valid");

    let started = Instant::now();
    let result = client.health();
    let elapsed = started.elapsed();

    assert!(result.is_err(), "truncated response must return Err");
    assert!(
        elapsed < Duration::from_millis(500),
        "must fail within timeout bounds"
    );

    server_thread.join().expect("server thread finished");
}

// 6. Malformed JSON Response Frame
#[test]
fn resilience_malformed_json_response() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        let _ = stream.read_exact(&mut token);
        let mut prefix = [0_u8; 4];
        let _ = stream.read_exact(&mut prefix);
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        let _ = stream.read_exact(&mut payload);

        // Send invalid JSON garbage payload
        let garbage = b"{ invalid json garbage !@#$%^&*() }";
        let declared = u32::try_from(garbage.len()).expect("fits u32");
        let _ = stream.write_all(&declared.to_be_bytes());
        let _ = stream.write_all(garbage);
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(500))
        .expect("client valid");

    let result = client.health();
    assert!(result.is_err(), "malformed json must return Err");
    match result.err().unwrap() {
        HostError::Protocol(ProtocolError::MalformedJson(_)) => {}
        other => panic!("expected ProtocolError::MalformedJson, got {other:?}"),
    }

    server_thread.join().expect("server thread finished");
}

// 7. Malformed Protocol Header (Unsupported Version)
#[test]
fn resilience_unsupported_protocol_version() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        let _ = stream.read_exact(&mut token);
        let mut prefix = [0_u8; 4];
        let _ = stream.read_exact(&mut prefix);
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        let _ = stream.read_exact(&mut payload);

        // Send response with unsupported protocol version 99
        let json_body = r#"{"protocolVersion":99,"requestId":"req-v99","result":{"type":"health","status":"ready","protocolVersion":99}}"#;
        let declared = u32::try_from(json_body.len()).expect("fits u32");
        let _ = stream.write_all(&declared.to_be_bytes());
        let _ = stream.write_all(json_body.as_bytes());
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(500))
        .expect("client valid");

    let result = client.health();
    assert!(result.is_err());
    match result.err().unwrap() {
        HostError::Protocol(ProtocolError::UnsupportedVersion(99)) => {}
        other => panic!("expected UnsupportedVersion(99), got {other:?}"),
    }

    server_thread.join().expect("server thread finished");
}

// 8. Invalid Response Shape (Both result and error null or both populated)
#[test]
fn resilience_invalid_response_shape() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        let _ = stream.read_exact(&mut token);
        let mut prefix = [0_u8; 4];
        let _ = stream.read_exact(&mut prefix);
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        let _ = stream.read_exact(&mut payload);

        // Send response with neither result nor error (shape violation)
        let json_body =
            r#"{"protocolVersion":1,"requestId":"req-bad-shape","result":null,"error":null}"#;
        let declared = u32::try_from(json_body.len()).expect("fits u32");
        let _ = stream.write_all(&declared.to_be_bytes());
        let _ = stream.write_all(json_body.as_bytes());
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(500))
        .expect("client valid");

    let result = client.health();
    assert!(result.is_err());
    match result.err().unwrap() {
        HostError::Protocol(ProtocolError::InvalidResponseShape) => {}
        other => panic!("expected InvalidResponseShape, got {other:?}"),
    }

    server_thread.join().expect("server thread finished");
}

// 9. Subscription Stream Timeout on Idle
#[test]
fn resilience_subscription_stream_timeout() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(3))
            .expect("client valid");

    let subscription = client.subscribe().expect("subscription succeeds");

    // Attempt to read next event with a short timeout when no events are published
    let started = Instant::now();
    let next_event = subscription.read_next_event(Duration::from_millis(80));
    let elapsed = started.elapsed();

    assert!(
        next_event.is_err(),
        "should time out when no events are present"
    );
    assert!(
        elapsed >= Duration::from_millis(70),
        "waited at least the timeout duration"
    );
    assert!(
        elapsed < Duration::from_millis(300),
        "did not hang beyond timeout"
    );
}

// 10. Burst of Events over Subscription Stream
#[test]
fn stress_subscription_rapid_burst_events() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(5))
            .expect("client valid");

    let subscription = client.subscribe().expect("subscription succeeds");

    // Dispatch 20 projects sequentially to trigger 40 events
    let repo_dir = create_temp_git_repository();
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    let client_clone = client.clone();
    let path_clone = repo_path.clone();
    let worker_thread = thread::spawn(move || {
        for i in 0..15 {
            let req_id = format!("burst-open-{i}");
            let idemp_key = format!("burst-idemp-{i}");
            let _ = client_clone.open_project(&path_clone, &req_id, &idemp_key);
        }
    });

    let mut received_sequences = Vec::new();
    for _ in 0..30 {
        let event = subscription
            .read_next_event(Duration::from_secs(3))
            .expect("event received");
        received_sequences.push(event.sequence());
    }

    worker_thread.join().expect("worker finished");
    let _ = std::fs::remove_dir_all(&repo_dir);

    // Verify sequences are monotonically strictly increasing: 1, 2, 3, ...
    for (i, seq) in received_sequences.iter().enumerate() {
        assert_eq!(
            *seq,
            (i + 1) as u64,
            "sequences must be in exact ascending order"
        );
    }
}

// 11. Client Construction Validation
#[test]
fn client_construction_input_validation() {
    let endpoint = unique_endpoint();

    // Zero timeout must fail
    let zero_timeout =
        DaemonClient::new_with_timeout(endpoint.clone(), TOKEN.into(), Duration::ZERO);
    assert!(matches!(zero_timeout, Err(HostError::InvalidTimeout)));

    // Invalid hex token must fail
    let invalid_token = DaemonClient::new(endpoint.clone(), "not-a-hex-token".into());
    assert!(matches!(
        invalid_token,
        Err(HostError::Protocol(ProtocolError::InvalidLaunchToken))
    ));

    // Short token must fail
    let short_token = DaemonClient::new(endpoint, "001122".into());
    assert!(matches!(
        short_token,
        Err(HostError::Protocol(ProtocolError::InvalidLaunchToken))
    ));
}

// 12. Unexpected Response Payload in health()
#[test]
fn resilience_unexpected_response_payload_in_health() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        let _ = stream.read_exact(&mut token);
        let mut prefix = [0_u8; 4];
        let _ = stream.read_exact(&mut prefix);
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        let _ = stream.read_exact(&mut payload);

        // Send a ProjectOpened response instead of Health response
        let resp = ServerResponse::success(
            "req-mismatch",
            ResponsePayload::ProjectOpened {
                project: kesharon_protocol::ProjectSnapshot {
                    id: "p1".into(),
                    display_name: "Mismatch".into(),
                    canonical_root: "/mismatch".into(),
                    trusted: false,
                },
            },
        );
        let frame = encode_frame(&resp).unwrap();
        let _ = stream.write_all(&frame);
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(500))
        .expect("client valid");

    let result = client.health();
    assert!(result.is_err());
    match result.err().unwrap() {
        HostError::UnexpectedResponse => {}
        other => panic!("expected UnexpectedResponse, got {other:?}"),
    }

    server_thread.join().expect("server thread finished");
}

// 13. Unexpected Snapshot when reading next event
#[test]
fn resilience_unexpected_snapshot_in_read_next_event() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");

    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("client connects");
        let mut token = [0_u8; 64];
        let _ = stream.read_exact(&mut token);
        let mut prefix = [0_u8; 4];
        let _ = stream.read_exact(&mut prefix);
        let length = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0_u8; length];
        let _ = stream.read_exact(&mut payload);

        // 1. Send SubscriptionReady
        let ready = ServerResponse::success(
            "sub-req-1",
            ResponsePayload::SubscriptionReady {
                stream_id: "stream-test-1".into(),
            },
        );
        let _ = stream.write_all(&encode_frame(&ready).unwrap());

        // 2. Send Initial Snapshot
        let snapshot = StreamMessage::Snapshot {
            snapshot: kesharon_protocol::WorkspaceSnapshot::new("stream-test-1", 0, None, vec![]),
        };
        let _ = stream.write_all(&encode_frame(&snapshot).unwrap());

        // 3. Send a SECOND Snapshot instead of an Event
        let second_snapshot = StreamMessage::Snapshot {
            snapshot: kesharon_protocol::WorkspaceSnapshot::new("stream-test-1", 1, None, vec![]),
        };
        let _ = stream.write_all(&encode_frame(&second_snapshot).unwrap());
    });

    let client = DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(500))
        .expect("client valid");

    let subscription = client.subscribe().expect("subscription handshake succeeds");
    let next_event = subscription.read_next_event(Duration::from_millis(500));

    assert!(next_event.is_err());
    match next_event.err().unwrap() {
        HostError::UnexpectedResponse => {}
        other => panic!("expected UnexpectedResponse, got {other:?}"),
    }

    server_thread.join().expect("server thread finished");
}

// 14. execute_health Deadline Exceeded (Daemon not running)
#[test]
fn resilience_execute_health_deadline_exceeded() {
    tauri::async_runtime::block_on(async {
        let endpoint = unique_endpoint();
        // Server endpoint is generated but no server is bound/running

        let client =
            DaemonClient::new_with_timeout(endpoint, TOKEN.into(), Duration::from_millis(50))
                .expect("client valid");

        let state = DaemonState::new_with_client(client);
        let started = Instant::now();
        let result = execute_health(&state).await;
        let elapsed = started.elapsed();

        assert!(result.is_err(), "should fail after connection deadline");
        assert!(
            result.unwrap_err().contains("connection deadline"),
            "error message should indicate deadline exceeded"
        );
        assert!(
            elapsed >= Duration::from_millis(900),
            "waited through retries"
        );
    });
}

// 15. Large Valid Payload Transmission (Paths up to 4KB and large request IDs)
#[test]
fn stress_large_valid_payload_roundtrip() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("valid token"));
    let runtime = ServerRuntime::new(daemon);

    let server_endpoint = endpoint.clone();
    let _server_thread = thread::spawn(move || {
        let _ = runtime.run(&server);
    });

    let client =
        DaemonClient::new_with_timeout(server_endpoint, TOKEN.into(), Duration::from_secs(5))
            .expect("client valid");

    let repo_dir = create_temp_git_repository();
    let repo_path = repo_dir.to_str().expect("valid path").to_owned();

    // Test with realistic long paths, request IDs, and idempotency keys
    for i in 0..5 {
        let req_id = format!("large-req-{}-{}", i, "x".repeat(128));
        let idemp_key = format!("large-idemp-{}-{}", i, "y".repeat(128));

        let res = client.open_project(&repo_path, &req_id, &idemp_key);
        assert!(
            res.is_ok(),
            "open_project with long request/idemp key should succeed"
        );
        let res = res.unwrap();
        assert_eq!(res.request_id(), req_id);
        assert!(matches!(
            res.result(),
            Some(ResponsePayload::ProjectOpened { .. })
        ));
    }

    let _ = std::fs::remove_dir_all(&repo_dir);
}
