use std::thread;

use kesharon_daemon::Daemon;
use kesharon_desktop_host::DaemonClient;
use kesharon_ipc::{LocalEndpoint, LocalServer};
use kesharon_protocol::{HealthStatus, LaunchToken, PROTOCOL_VERSION};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn unique_endpoint() -> LocalEndpoint {
    #[cfg(windows)]
    let value = format!(
        "kesharon-host-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the Unix epoch")
            .as_nanos()
    );

    #[cfg(unix)]
    let value = std::env::temp_dir()
        .join(format!(
            "kesharon-host-test-{}-{}.sock",
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
