use std::io::Cursor;

use kesharon_daemon::{Daemon, DaemonError};
use kesharon_protocol::{
    ClientRequest, HealthStatus, LaunchToken, PROTOCOL_VERSION, RequestMethod, ResponsePayload,
    ServerResponse, decode_server_response_frame, encode_frame,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

#[test]
fn connection_reads_one_frame_and_writes_one_response() {
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let request = ClientRequest::new("request-1", RequestMethod::Health, None)
        .expect("health requests are valid");
    let input = encode_frame(&request).expect("the request frame is valid");
    let mut connection = Cursor::new(input);
    let mut output = Vec::new();

    daemon
        .serve_connection(TOKEN, &mut connection, &mut output)
        .expect("the connection is valid");

    assert_eq!(
        decode_server_response_frame(&output).expect("the response frame is valid"),
        ServerResponse::success(
            "request-1",
            ResponsePayload::Health {
                status: HealthStatus::Ready,
                protocol_version: PROTOCOL_VERSION,
            },
        )
    );
}

#[test]
fn connection_rejects_oversized_length_before_reading_a_payload() {
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let mut connection = Cursor::new((8_u32 * 1024 * 1024 + 1).to_be_bytes());
    let mut output = Vec::new();

    let result = daemon.serve_connection(TOKEN, &mut connection, &mut output);

    assert_eq!(
        result,
        Err(DaemonError::FrameTooLarge {
            declared: 8 * 1024 * 1024 + 1,
            maximum: 8 * 1024 * 1024,
        })
    );
    assert!(output.is_empty());
}
