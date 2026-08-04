use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kesharon_daemon::Daemon;
use kesharon_desktop_host::DaemonClient;
use kesharon_ipc::{LocalEndpoint, LocalServer};
use kesharon_protocol::{HealthStatus, LaunchToken, PROTOCOL_VERSION};

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
