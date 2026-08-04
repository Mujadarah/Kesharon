use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use kesharon_ipc::{
    LocalEndpoint, LocalStream, connect, connect_with_timeout, read_exact_with_timeout,
    write_all_with_timeout,
};
use kesharon_protocol::{
    CancellationOutcome, ClientRequest, ErrorCode, HealthStatus, PROTOCOL_VERSION, RequestMethod,
    ResponsePayload, ServerResponse, StreamMessage, decode_server_response_frame,
    decode_stream_message_frame, encode_frame,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn unique_endpoint() -> LocalEndpoint {
    #[cfg(windows)]
    let value = format!(
        "kesharon-process-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the Unix epoch")
            .as_nanos()
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "kesharon-process-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is after the Unix epoch")
                .as_nanos()
        ))
        .to_string_lossy()
        .into_owned();

    LocalEndpoint::new(value).expect("the generated endpoint is valid")
}

struct DaemonProcess {
    child: Child,
    endpoint: LocalEndpoint,
}

impl DaemonProcess {
    fn start(extra_env: Option<(&str, &str)>) -> Self {
        let endpoint = unique_endpoint();
        let mut command = Command::new(env!("CARGO_BIN_EXE_kesharon-daemon"));
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
        assert_eq!(ready, "READY 1\n");
        Self { child, endpoint }
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn open_authenticated_connection(endpoint: &LocalEndpoint, request: &ClientRequest) -> LocalStream {
    let stream = connect_with_timeout(endpoint, Duration::from_secs(2))
        .expect("the daemon endpoint accepts a connection");
    write_all_with_timeout(&stream, TOKEN.as_bytes(), Duration::from_secs(2))
        .expect("authentication is writable");
    let frame = encode_frame(request).expect("the request frame is valid");
    write_all_with_timeout(&stream, &frame, Duration::from_secs(2))
        .expect("the request is writable");
    stream
}

fn read_frame(stream: &LocalStream) -> Vec<u8> {
    let mut prefix = [0_u8; 4];
    read_exact_with_timeout(stream, &mut prefix, Duration::from_secs(2))
        .expect("the response prefix is readable");
    let length = u32::from_be_bytes(prefix) as usize;
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&prefix);
    frame.resize(length + 4, 0);
    read_exact_with_timeout(stream, &mut frame[4..], Duration::from_secs(2))
        .expect("the response payload is readable");
    frame
}

fn request(endpoint: &LocalEndpoint, request: &ClientRequest) -> ServerResponse {
    let stream = open_authenticated_connection(endpoint, request);
    decode_server_response_frame(&read_frame(&stream)).expect("the response is valid")
}

fn workspace_root() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root exists")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn daemon_process_reads_token_from_stdin_and_serves_the_local_endpoint() {
    let endpoint = unique_endpoint();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kesharon-daemon"))
        .args(["--endpoint", endpoint.as_str(), "--once"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the daemon binary starts");

    let mut stdin = child.stdin.take().expect("daemon stdin is piped");
    writeln!(stdin, "{TOKEN}").expect("the launch token is writable");
    drop(stdin);

    let stdout = child.stdout.take().expect("daemon stdout is piped");
    let mut stdout = BufReader::new(stdout);
    let mut ready = String::new();
    stdout
        .read_line(&mut ready)
        .expect("the readiness line is readable");
    assert_eq!(ready, "READY 1\n");

    let request = ClientRequest::new("request-process-1", RequestMethod::Health, None)
        .expect("health requests are valid");
    let request_frame = encode_frame(&request).expect("the request frame is valid");
    let mut client = connect(&endpoint).expect("the daemon endpoint is listening");
    client
        .write_all(TOKEN.as_bytes())
        .expect("the authentication preface is writable");
    client
        .write_all(&request_frame)
        .expect("the request frame is writable");

    let mut prefix = [0_u8; 4];
    client
        .read_exact(&mut prefix)
        .expect("the response prefix is readable");
    let length = u32::from_be_bytes(prefix) as usize;
    let mut response_frame = Vec::with_capacity(length + 4);
    response_frame.extend_from_slice(&prefix);
    response_frame.resize(length + 4, 0);
    client
        .read_exact(&mut response_frame[4..])
        .expect("the response payload is readable");

    assert_eq!(
        decode_server_response_frame(&response_frame).expect("the response is valid"),
        ServerResponse::success(
            "request-process-1",
            ResponsePayload::Health {
                status: HealthStatus::Ready,
                protocol_version: PROTOCOL_VERSION,
            },
        )
    );

    let status = child.wait().expect("the daemon process exits");
    assert!(status.success());
}

#[test]
fn daemon_process_keeps_a_subscription_while_serving_health_and_open_requests() {
    let daemon = DaemonProcess::start(None);
    let subscribe = ClientRequest::new("request-subscribe", RequestMethod::SubscribeEvents, None)
        .expect("subscription request is valid");
    let subscription = open_authenticated_connection(&daemon.endpoint, &subscribe);
    let ready = decode_server_response_frame(&read_frame(&subscription))
        .expect("subscription-ready response is valid");
    assert!(matches!(
        ready.result(),
        Some(ResponsePayload::SubscriptionReady { .. })
    ));
    assert!(matches!(
        decode_stream_message_frame(&read_frame(&subscription)).expect("snapshot is valid"),
        StreamMessage::Snapshot { .. }
    ));

    let health = request(
        &daemon.endpoint,
        &ClientRequest::new("request-health", RequestMethod::Health, None)
            .expect("health request is valid"),
    );
    assert!(matches!(
        health.result(),
        Some(ResponsePayload::Health {
            status: HealthStatus::Ready,
            ..
        })
    ));

    let open = request(
        &daemon.endpoint,
        &ClientRequest::new(
            "request-open",
            RequestMethod::OpenProject {
                path: workspace_root(),
            },
            Some("open-key".into()),
        )
        .expect("open request is valid"),
    );
    assert!(matches!(
        open.result(),
        Some(ResponsePayload::ProjectOpened { .. })
    ));
    assert!(matches!(
        decode_stream_message_frame(&read_frame(&subscription)).expect("started event is valid"),
        StreamMessage::Event { .. }
    ));
    assert!(matches!(
        decode_stream_message_frame(&read_frame(&subscription)).expect("terminal event is valid"),
        StreamMessage::Event { .. }
    ));
}

#[test]
fn cancellation_arrives_on_a_second_connection_while_process_open_is_blocked() {
    let barrier = std::env::temp_dir().join(format!(
        "kesharon-open-barrier-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the Unix epoch")
            .as_nanos()
    ));
    let barrier_text = barrier.to_string_lossy().into_owned();
    let entered = barrier.with_extension("entered");
    let daemon = DaemonProcess::start(Some((
        "KESHARON_TEST_OPEN_BLOCK_UNTIL",
        barrier_text.as_str(),
    )));
    let open = ClientRequest::new(
        "request-open",
        RequestMethod::OpenProject {
            path: workspace_root(),
        },
        Some("open-key".into()),
    )
    .expect("open request is valid");
    let open_connection = open_authenticated_connection(&daemon.endpoint, &open);

    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.exists() {
        assert!(
            Instant::now() < deadline,
            "open operation did not reach barrier"
        );
        std::thread::yield_now();
    }
    let cancel = request(
        &daemon.endpoint,
        &ClientRequest::new(
            "request-cancel",
            RequestMethod::CancelRequest {
                target_request_id: "request-open".into(),
            },
            Some("cancel-key".into()),
        )
        .expect("cancel request is valid"),
    );
    assert!(matches!(
        cancel.result(),
        Some(ResponsePayload::Cancellation {
            outcome: CancellationOutcome::Accepted,
            ..
        })
    ));
    fs::write(&barrier, b"release").expect("barrier can be released");
    let open = decode_server_response_frame(&read_frame(&open_connection))
        .expect("cancelled open response is valid");
    assert_eq!(
        open.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::OperationCancelled)
    );
    let _ = fs::remove_file(&entered);
    let _ = fs::remove_file(&barrier);
}

#[test]
fn authenticated_ninth_concurrent_connection_receives_server_busy() {
    let daemon = DaemonProcess::start(None);
    let mut occupied = Vec::new();
    for _ in 0..8 {
        let stream = connect_with_timeout(&daemon.endpoint, Duration::from_secs(2))
            .expect("connection slot is accepted");
        write_all_with_timeout(&stream, TOKEN.as_bytes(), Duration::from_secs(2))
            .expect("authentication is writable");
        occupied.push(stream);
    }

    let busy = request(
        &daemon.endpoint,
        &ClientRequest::new("request-busy", RequestMethod::Health, None)
            .expect("health request is valid"),
    );
    assert_eq!(
        busy.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::ServerBusy)
    );
    assert_eq!(busy.request_id(), "request-busy");
    drop(occupied);
}

#[test]
fn repeated_subscriber_replacement_releases_connection_permits() {
    let daemon = DaemonProcess::start(None);
    let mut subscriptions = Vec::new();
    for index in 0..12 {
        let subscribe = ClientRequest::new(
            format!("request-subscribe-{index}"),
            RequestMethod::SubscribeEvents,
            None,
        )
        .expect("subscription request is valid");
        let stream = open_authenticated_connection(&daemon.endpoint, &subscribe);
        let ready = decode_server_response_frame(&read_frame(&stream))
            .expect("subscription-ready response is valid");
        assert!(matches!(
            ready.result(),
            Some(ResponsePayload::SubscriptionReady { .. })
        ));
        assert!(matches!(
            decode_stream_message_frame(&read_frame(&stream)).expect("snapshot is valid"),
            StreamMessage::Snapshot { .. }
        ));
        subscriptions.push(stream);
    }

    let health = request(
        &daemon.endpoint,
        &ClientRequest::new(
            "request-health-after-replacements",
            RequestMethod::Health,
            None,
        )
        .expect("health request is valid"),
    );
    assert!(matches!(
        health.result(),
        Some(ResponsePayload::Health { .. })
    ));
    drop(subscriptions);
}
