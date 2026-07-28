use std::io::{Read, Write};
use std::thread;

use kesharon_daemon::Daemon;
use kesharon_ipc::{LocalEndpoint, LocalServer, connect};
use kesharon_protocol::{
    ClientRequest, HealthStatus, LaunchToken, PROTOCOL_VERSION, RequestMethod, ResponsePayload,
    ServerResponse, decode_server_response_frame, encode_frame,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn unique_endpoint() -> LocalEndpoint {
    #[cfg(windows)]
    let value = format!(
        "kesharon-daemon-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the Unix epoch")
            .as_nanos()
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "kesharon-daemon-test-{}-{}.sock",
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

#[test]
fn authenticated_local_socket_dispatches_a_health_frame() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let server_thread = thread::spawn(move || {
        daemon
            .serve_local_connection(&server)
            .expect("the client request is valid");
    });

    let request = ClientRequest::new("request-local-1", RequestMethod::Health, None)
        .expect("health requests are valid");
    let request_frame = encode_frame(&request).expect("the request is encodable");
    let mut client = connect(&endpoint).expect("the server is listening");
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
            "request-local-1",
            ResponsePayload::Health {
                status: HealthStatus::Ready,
                protocol_version: PROTOCOL_VERSION,
            },
        )
    );
    server_thread
        .join()
        .expect("the server thread does not panic");
}
