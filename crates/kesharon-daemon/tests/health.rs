use kesharon_daemon::{Daemon, DaemonError};
use kesharon_protocol::{
    ClientRequest, HealthStatus, LaunchToken, PROTOCOL_VERSION, RequestMethod, ResponsePayload,
    ServerResponse, decode_server_response_frame, encode_frame,
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

#[test]
fn authenticated_health_request_returns_ready_response() {
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let request = ClientRequest::new("request-1", RequestMethod::Health, None)
        .expect("health requests are valid");
    let request_frame = encode_frame(&request).expect("the request frame is valid");

    let response_frame = daemon
        .handle_authenticated_frame(TOKEN, &request_frame)
        .expect("the token and request are valid");
    let response =
        decode_server_response_frame(&response_frame).expect("the daemon response is valid");

    assert_eq!(
        response,
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
fn invalid_launch_token_is_rejected_before_request_dispatch() {
    let daemon = Daemon::new(LaunchToken::parse_hex(TOKEN).expect("the fixture token is valid"));
    let request = ClientRequest::new("request-2", RequestMethod::Health, None)
        .expect("health requests are valid");
    let request_frame = encode_frame(&request).expect("the request frame is valid");

    let result = daemon.handle_authenticated_frame(
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        &request_frame,
    );

    assert_eq!(result, Err(DaemonError::AuthenticationFailed));
}
