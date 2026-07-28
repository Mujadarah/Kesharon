use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use kesharon_ipc::{LocalEndpoint, connect};
use kesharon_protocol::{
    ClientRequest, HealthStatus, PROTOCOL_VERSION, RequestMethod, ResponsePayload, ServerResponse,
    decode_server_response_frame, encode_frame,
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
