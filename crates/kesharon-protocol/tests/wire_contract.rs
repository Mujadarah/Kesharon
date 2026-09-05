use kesharon_protocol::{
    CancellationOutcome, ClientRequest, DaemonEventPayload, ErrorCode, HealthStatus, LaunchToken,
    MAX_FRAME_BYTES, PROTOCOL_VERSION, ProtocolError, RequestMethod, ResourceBudgetSnapshot,
    ResponsePayload, ServerResponse, StreamMessage, TaskSnapshot, WorkspaceSnapshot,
    decode_client_request_frame, decode_server_response_frame, decode_stream_message_frame,
    encode_frame,
};
use serde_json::Value;

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

fn session_fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../packages/protocol/fixtures/session-messages.json"
    ))
    .expect("session fixture JSON is valid")
}

fn frame_fixture(value: &Value) -> Vec<u8> {
    encode_frame(value).expect("fixture is encodable")
}

#[test]
fn shared_open_and_cancel_requests_decode_with_idempotency() {
    let fixtures = session_fixtures();
    let health = decode_client_request_frame(&frame_fixture(&fixtures["healthRequest"]))
        .expect("health fixture is valid");
    let open = decode_client_request_frame(&frame_fixture(&fixtures["openProjectRequest"]))
        .expect("open-project fixture is valid");
    let cancel = decode_client_request_frame(&frame_fixture(&fixtures["cancelRequest"]))
        .expect("cancel fixture is valid");
    let subscribe =
        decode_client_request_frame(&frame_fixture(&fixtures["subscribeEventsRequest"]))
            .expect("subscribe fixture is valid");

    assert!(matches!(health.method(), RequestMethod::Health));
    assert!(matches!(
        open.method(),
        RequestMethod::OpenProject { path } if path == r"D:\code\kesharon"
    ));
    assert_eq!(open.idempotency_key(), Some("open-key-1"));
    assert!(matches!(
        cancel.method(),
        RequestMethod::CancelRequest { target_request_id }
            if target_request_id == "request-open-1"
    ));
    assert_eq!(cancel.idempotency_key(), Some("cancel-key-1"));
    assert!(matches!(subscribe.method(), RequestMethod::SubscribeEvents));
}

#[test]
fn shared_snapshot_and_event_messages_decode() {
    let fixtures = session_fixtures();
    let snapshot = decode_stream_message_frame(&frame_fixture(&fixtures["snapshotMessage"]))
        .expect("snapshot fixture is valid");
    let event = decode_stream_message_frame(&frame_fixture(&fixtures["eventMessage"]))
        .expect("event fixture is valid");
    for name in [
        "operationStartedMessage",
        "operationCancelledMessage",
        "failureEventMessage",
    ] {
        decode_stream_message_frame(&frame_fixture(&fixtures[name]))
            .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
    }

    assert!(matches!(
        snapshot,
        StreamMessage::Snapshot { snapshot }
            if snapshot.stream_id() == "stream-1"
                && snapshot.last_sequence() == 4
                && snapshot.project().is_some()
                && snapshot.active_operations().len() == 1
    ));
    assert!(matches!(
        event,
        StreamMessage::Event { event }
            if event.sequence() == 5
                && matches!(event.payload(), DaemonEventPayload::ProjectOpened { .. })
    ));
}

#[test]
fn cancellation_response_and_structured_error_round_trip() {
    let cancelled = ServerResponse::success(
        "request-cancel-1",
        ResponsePayload::Cancellation {
            target_request_id: "request-open-1".into(),
            outcome: CancellationOutcome::Accepted,
        },
    );
    let failure = ServerResponse::failure(
        "request-open-1",
        ErrorCode::NotGitRepository,
        "Selected directory is not a Git worktree",
    );

    let cancelled = decode_server_response_frame(
        &encode_frame(&cancelled).expect("cancellation response is encodable"),
    )
    .expect("cancellation response decodes");
    let failure = decode_server_response_frame(
        &encode_frame(&failure).expect("failure response is encodable"),
    )
    .expect("failure response decodes");

    assert!(matches!(
        cancelled.result(),
        Some(ResponsePayload::Cancellation {
            outcome: CancellationOutcome::Accepted,
            ..
        })
    ));
    assert_eq!(
        failure.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::NotGitRepository)
    );
}

#[test]
fn every_shared_response_and_failure_event_decodes_with_typed_error_codes() {
    let fixtures = session_fixtures();
    for name in [
        "healthResponse",
        "projectOpenedResponse",
        "cancellationResponse",
        "subscriptionReadyResponse",
        "failureResponse",
    ] {
        decode_server_response_frame(&frame_fixture(&fixtures[name]))
            .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
    }

    let failure = decode_server_response_frame(&frame_fixture(&fixtures["failureResponse"]))
        .expect("failure fixture decodes");
    assert_eq!(
        failure.error().map(kesharon_protocol::ErrorPayload::code),
        Some(ErrorCode::NotGitRepository)
    );

    let event = decode_stream_message_frame(&frame_fixture(&fixtures["failureEventMessage"]))
        .expect("failure event fixture decodes");
    assert!(matches!(
        event,
        StreamMessage::Event { event }
            if matches!(
                event.payload(),
                DaemonEventPayload::OperationFailed {
                    code: ErrorCode::NotGitRepository,
                    ..
                }
            )
    ));
}

#[test]
fn cancel_requests_require_idempotency() {
    assert_eq!(
        ClientRequest::new(
            "request-cancel-2",
            RequestMethod::CancelRequest {
                target_request_id: "request-open-2".into(),
            },
            None,
        ),
        Err(ProtocolError::MissingIdempotencyKey)
    );
}

#[test]
fn workspace_snapshot_with_active_task_round_trips() {
    let budget = ResourceBudgetSnapshot {
        max_memory_bytes: 1024 * 1024 * 1024,
        max_disk_write_bytes: 512 * 1024 * 1024,
        max_concurrent_tools: 4,
        max_prompt_tokens: Some(100_000),
        max_completion_tokens: Some(4_000),
        max_cost_micros: Some(500_000),
    };
    let task = TaskSnapshot {
        id: "task-01".to_string(),
        goal: "Implement Task 1".to_string(),
        state: "running".to_string(),
        execution_mode: "autonomous".to_string(),
        budget,
    };
    let snapshot = WorkspaceSnapshot::new("stream-task-1", 10, None, vec![], Some(task.clone()));

    assert_eq!(snapshot.active_task(), Some(&task));

    let message = StreamMessage::Snapshot {
        snapshot: snapshot.clone(),
    };
    let frame = encode_frame(&message).expect("message is encodable");
    let decoded = decode_stream_message_frame(&frame).expect("snapshot frame decodes");

    assert_eq!(decoded, message);

    let json = serde_json::to_string(&snapshot).expect("snapshot serializes to json");
    assert!(json.contains("\"activeTask\""));
    assert!(json.contains("\"maxPromptTokens\":100000"));

    let deserialized: WorkspaceSnapshot =
        serde_json::from_str(&json).expect("snapshot deserializes from json");
    assert_eq!(deserialized, snapshot);
}

#[test]
fn workspace_snapshot_without_active_task_omits_field_and_round_trips() {
    let snapshot = WorkspaceSnapshot::new("stream-task-2", 0, None, vec![], None);

    assert_eq!(snapshot.active_task(), None);

    let json = serde_json::to_string(&snapshot).expect("snapshot serializes to json");
    assert!(!json.contains("\"activeTask\""));

    let deserialized: WorkspaceSnapshot =
        serde_json::from_str(&json).expect("snapshot deserializes from json without activeTask");
    assert_eq!(deserialized, snapshot);
    assert!(deserialized.active_task().is_none());

    let json_with_null = json.replacen(
        "\"streamId\":\"stream-task-2\"",
        "\"streamId\":\"stream-task-2\",\"activeTask\":null",
        1,
    );
    let deserialized_null: WorkspaceSnapshot =
        serde_json::from_str(&json_with_null).expect("snapshot deserializes with activeTask: null");
    assert_eq!(deserialized_null, snapshot);
    assert!(deserialized_null.active_task().is_none());
}

#[test]
fn resource_budget_snapshot_round_trips_optional_fields() {
    let minimal_budget = ResourceBudgetSnapshot {
        max_memory_bytes: 256 * 1024 * 1024,
        max_disk_write_bytes: 64 * 1024 * 1024,
        max_concurrent_tools: 2,
        max_prompt_tokens: None,
        max_completion_tokens: None,
        max_cost_micros: None,
    };

    let json = serde_json::to_string(&minimal_budget).expect("budget serializes");
    assert!(!json.contains("maxPromptTokens"));
    assert!(!json.contains("maxCompletionTokens"));
    assert!(!json.contains("maxCostMicros"));

    let deserialized: ResourceBudgetSnapshot =
        serde_json::from_str(&json).expect("budget deserializes");
    assert_eq!(deserialized, minimal_budget);

    let full_budget = ResourceBudgetSnapshot {
        max_memory_bytes: 512 * 1024 * 1024,
        max_disk_write_bytes: 128 * 1024 * 1024,
        max_concurrent_tools: 8,
        max_prompt_tokens: Some(50_000),
        max_completion_tokens: Some(2_000),
        max_cost_micros: Some(10_000),
    };

    let json_full = serde_json::to_string(&full_budget).expect("full budget serializes");
    let deserialized_full: ResourceBudgetSnapshot =
        serde_json::from_str(&json_full).expect("full budget deserializes");
    assert_eq!(deserialized_full, full_budget);
}
