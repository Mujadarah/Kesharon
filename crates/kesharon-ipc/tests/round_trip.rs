use std::io::{Read, Write};
use std::thread;

use kesharon_ipc::{LocalEndpoint, LocalServer, connect};

fn unique_endpoint() -> LocalEndpoint {
    #[cfg(windows)]
    let value = format!(
        "kesharon-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the Unix epoch")
            .as_nanos()
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "kesharon-test-{}-{}.sock",
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
fn local_socket_round_trip_uses_a_cross_platform_stream() {
    let endpoint = unique_endpoint();
    let server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let server_thread = thread::spawn(move || {
        let mut stream = server.accept().expect("the client connects");
        let mut input = [0_u8; 4];
        stream
            .read_exact(&mut input)
            .expect("the request is readable");
        assert_eq!(&input, b"ping");
        stream.write_all(b"pong").expect("the response is writable");
    });

    let mut client = connect(&endpoint).expect("the server is listening");
    client.write_all(b"ping").expect("the request is writable");
    let mut response = [0_u8; 4];
    client
        .read_exact(&mut response)
        .expect("the response is readable");

    assert_eq!(&response, b"pong");
    server_thread
        .join()
        .expect("the server thread does not panic");
}

#[test]
fn blank_endpoint_is_rejected() {
    assert!(LocalEndpoint::new("  ").is_err());
}

#[cfg(unix)]
#[test]
fn unix_socket_is_accessible_only_to_its_owner() {
    use std::os::unix::fs::PermissionsExt;

    let endpoint = unique_endpoint();
    let _server = LocalServer::bind(&endpoint).expect("the endpoint is available");
    let mode = std::fs::metadata(endpoint.as_str())
        .expect("the socket exists")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
}
