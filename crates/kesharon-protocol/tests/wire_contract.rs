use kesharon_protocol::{
    ClientRequest, HealthStatus, LaunchToken, MAX_FRAME_BYTES, PROTOCOL_VERSION, ProtocolError,
    RequestMethod, ResponsePayload, ServerResponse, decode_client_request_frame,
    decode_server_response_frame, encode_frame,
};

#[test]
fn health_request_round_trips_through_length_prefixed_json() {
    let request = ClientRequest::new("request-1", RequestMethod::Health, None)
        .expect("health does not require idempotency");

    let frame = encode_frame(&request).expect("the request is encodable");
    let declared_length = u32::from_be_bytes(frame[..4].try_into().expect("four-byte prefix"));

    assert_eq!(declared_length as usize, frame.len() - 4);
    assert_eq!(
        decode_client_request_frame(&frame).expect("the frame is valid"),
        request
    );
    assert_eq!(request.protocol_version(), PROTOCOL_VERSION);
}

#[test]
fn mutating_request_requires_an_idempotency_key() {
    let result = ClientRequest::new(
        "request-2",
        RequestMethod::OpenProject {
            path: "C:\\code\\project".into(),
        },
        None,
    );

    assert_eq!(result, Err(ProtocolError::MissingIdempotencyKey));
}

#[test]
fn decoder_rejects_unsupported_protocol_versions() {
    let payload = br#"{"protocolVersion":2,"requestId":"request-3","method":{"type":"health"},"idempotencyKey":null}"#;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("the fixture payload fits in a protocol frame")
            .to_be_bytes(),
    );
    frame.extend_from_slice(payload);

    assert_eq!(
        decode_client_request_frame(&frame),
        Err(ProtocolError::UnsupportedVersion(2))
    );
}

#[test]
fn decoder_rejects_frames_over_the_eight_mebibyte_limit() {
    let oversized = u32::try_from(MAX_FRAME_BYTES + 1).expect("limit fits in u32");
    let frame = oversized.to_be_bytes();

    assert_eq!(
        decode_client_request_frame(&frame),
        Err(ProtocolError::FrameTooLarge {
            declared: MAX_FRAME_BYTES + 1,
            maximum: MAX_FRAME_BYTES,
        })
    );
}

#[test]
fn decoder_rejects_incomplete_and_trailing_frames() {
    assert_eq!(
        decode_client_request_frame(&[0, 0, 0]),
        Err(ProtocolError::IncompleteLengthPrefix)
    );

    let request = ClientRequest::new("request-4", RequestMethod::Health, None)
        .expect("health does not require idempotency");
    let mut frame = encode_frame(&request).expect("the request is encodable");
    frame.push(0);

    assert_eq!(
        decode_client_request_frame(&frame),
        Err(ProtocolError::FrameLengthMismatch {
            declared: frame.len() - 5,
            actual: frame.len() - 4,
        })
    );
}

#[test]
fn launch_token_accepts_exactly_256_bits_of_hex() {
    let token =
        LaunchToken::parse_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
            .expect("64 hexadecimal characters encode 256 bits");

    assert!(token.matches_hex("000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F"));
    assert_eq!(
        LaunchToken::parse_hex("abcd"),
        Err(ProtocolError::InvalidLaunchToken)
    );
    assert_eq!(
        LaunchToken::parse_hex("zz0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",),
        Err(ProtocolError::InvalidLaunchToken)
    );
}

#[test]
fn successful_response_round_trips_without_an_error_branch() {
    let response = ServerResponse::success(
        "request-5",
        ResponsePayload::Health {
            status: HealthStatus::Ready,
            protocol_version: PROTOCOL_VERSION,
        },
    );

    let frame = encode_frame(&response).expect("the response is encodable");
    let decoded = decode_server_response_frame(&frame).expect("the response frame is valid");

    assert_eq!(decoded, response);
    assert!(decoded.result().is_some());
    assert!(decoded.error().is_none());
}
