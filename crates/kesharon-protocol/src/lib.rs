#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{Debug, Display, Formatter};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientRequest {
    protocol_version: u16,
    request_id: String,
    method: RequestMethod,
    idempotency_key: Option<String>,
}

impl ClientRequest {
    pub fn new(
        request_id: impl Into<String>,
        method: RequestMethod,
        idempotency_key: Option<String>,
    ) -> Result<Self, ProtocolError> {
        if method.requires_idempotency_key() && idempotency_key.as_deref().is_none_or(str::is_empty)
        {
            return Err(ProtocolError::MissingIdempotencyKey);
        }

        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            method,
            idempotency_key,
        })
    }

    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn method(&self) -> &RequestMethod {
        &self.method
    }

    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum RequestMethod {
    Health,
    OpenProject { path: String },
    CancelRequest { target_request_id: String },
    SubscribeEvents,
}

impl RequestMethod {
    const fn requires_idempotency_key(&self) -> bool {
        matches!(self, Self::OpenProject { .. } | Self::CancelRequest { .. })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerResponse {
    protocol_version: u16,
    request_id: String,
    result: Option<ResponsePayload>,
    error: Option<ErrorPayload>,
}

impl ServerResponse {
    pub fn success(request_id: impl Into<String>, result: ResponsePayload) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            result: Some(result),
            error: None,
        }
    }

    pub const fn result(&self) -> Option<&ResponsePayload> {
        self.result.as_ref()
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn error(&self) -> Option<&ErrorPayload> {
        self.error.as_ref()
    }

    pub fn failure(
        request_id: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            result: None,
            error: Some(ErrorPayload {
                code,
                message: message.into(),
            }),
        }
    }

    #[must_use]
    pub fn with_request_id(&self, request_id: impl Into<String>) -> Self {
        let mut replay = self.clone();
        replay.request_id = request_id.into();
        replay
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum ResponsePayload {
    Health {
        status: HealthStatus,
        protocol_version: u16,
    },
    ProjectOpened {
        project: ProjectSnapshot,
    },
    Cancellation {
        target_request_id: String,
        outcome: CancellationOutcome,
    },
    SubscriptionReady {
        stream_id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CancellationOutcome {
    Accepted,
    AlreadyFinished,
    NotFound,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    pub id: String,
    pub display_name: String,
    pub canonical_root: String,
    pub trusted: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    OpenProject,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationStatus {
    Running,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSnapshot {
    pub request_id: String,
    pub kind: OperationKind,
    pub status: OperationStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    stream_id: String,
    last_sequence: u64,
    project: Option<ProjectSnapshot>,
    active_operations: Vec<OperationSnapshot>,
}

impl WorkspaceSnapshot {
    pub fn new(
        stream_id: impl Into<String>,
        last_sequence: u64,
        project: Option<ProjectSnapshot>,
        active_operations: Vec<OperationSnapshot>,
    ) -> Self {
        Self {
            stream_id: stream_id.into(),
            last_sequence,
            project,
            active_operations,
        }
    }

    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub const fn project(&self) -> Option<&ProjectSnapshot> {
        self.project.as_ref()
    }

    pub fn active_operations(&self) -> &[OperationSnapshot] {
        &self.active_operations
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum DaemonEventPayload {
    OperationStarted {
        request_id: String,
        kind: OperationKind,
    },
    ProjectOpened {
        request_id: String,
        project: ProjectSnapshot,
    },
    OperationCancelled {
        request_id: String,
    },
    OperationFailed {
        request_id: String,
        code: ErrorCode,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonEvent {
    protocol_version: u16,
    stream_id: String,
    sequence: u64,
    payload: DaemonEventPayload,
}

impl DaemonEvent {
    pub fn new(
        stream_id: impl Into<String>,
        sequence: u64,
        payload: DaemonEventPayload,
    ) -> Result<Self, ProtocolError> {
        if sequence == 0 {
            return Err(ProtocolError::InvalidEventSequence);
        }
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            stream_id: stream_id.into(),
            sequence,
            payload,
        })
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub const fn payload(&self) -> &DaemonEventPayload {
        &self.payload
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "messageType"
)]
pub enum StreamMessage {
    Snapshot { snapshot: WorkspaceSnapshot },
    Event { event: DaemonEvent },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthStatus {
    Ready,
    Degraded,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    InvalidRequest,
    NotGitRepository,
    ProjectPathUnavailable,
    OperationCancelled,
    RequestInProgress,
    ServerBusy,
    InternalError,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    code: ErrorCode,
    message: String,
}

impl ErrorPayload {
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = serde_json::to_vec(message)
        .map_err(|error| ProtocolError::MalformedJson(error.to_string()))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            declared: payload.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }

    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge {
        declared: payload.len(),
        maximum: MAX_FRAME_BYTES,
    })?;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_client_request_frame(frame: &[u8]) -> Result<ClientRequest, ProtocolError> {
    let request: ClientRequest = decode_frame(frame)?;
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(request.protocol_version));
    }
    if request.method.requires_idempotency_key()
        && request.idempotency_key.as_deref().is_none_or(str::is_empty)
    {
        return Err(ProtocolError::MissingIdempotencyKey);
    }
    Ok(request)
}

pub fn decode_server_response_frame(frame: &[u8]) -> Result<ServerResponse, ProtocolError> {
    let response: ServerResponse = decode_frame(frame)?;
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(response.protocol_version));
    }
    if response.result.is_some() == response.error.is_some() {
        return Err(ProtocolError::InvalidResponseShape);
    }
    Ok(response)
}

pub fn decode_stream_message_frame(frame: &[u8]) -> Result<StreamMessage, ProtocolError> {
    let message: StreamMessage = decode_frame(frame)?;
    if let StreamMessage::Event { event } = &message {
        if event.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(event.protocol_version));
        }
        if event.sequence == 0 {
            return Err(ProtocolError::InvalidEventSequence);
        }
    }
    Ok(message)
}

fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<T, ProtocolError> {
    if frame.len() < 4 {
        return Err(ProtocolError::IncompleteLengthPrefix);
    }

    let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if declared > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            declared,
            maximum: MAX_FRAME_BYTES,
        });
    }

    let actual = frame.len() - 4;
    if actual != declared {
        return Err(ProtocolError::FrameLengthMismatch { declared, actual });
    }

    serde_json::from_slice(&frame[4..])
        .map_err(|error| ProtocolError::MalformedJson(error.to_string()))
}

#[derive(Clone, Eq, PartialEq)]
pub struct LaunchToken([u8; 32]);

impl LaunchToken {
    pub fn parse_hex(value: &str) -> Result<Self, ProtocolError> {
        if value.len() != 64 {
            return Err(ProtocolError::InvalidLaunchToken);
        }

        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
                .map_err(|_| ProtocolError::InvalidLaunchToken)?;
        }
        Ok(Self(bytes))
    }

    pub fn matches_hex(&self, candidate: &str) -> bool {
        let Ok(candidate) = Self::parse_hex(candidate) else {
            return false;
        };
        self.0
            .iter()
            .zip(candidate.0)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

impl Debug for LaunchToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LaunchToken([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    IncompleteLengthPrefix,
    FrameTooLarge { declared: usize, maximum: usize },
    FrameLengthMismatch { declared: usize, actual: usize },
    MalformedJson(String),
    UnsupportedVersion(u16),
    MissingIdempotencyKey,
    InvalidLaunchToken,
    InvalidResponseShape,
    InvalidEventSequence,
}

impl Display for ProtocolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompleteLengthPrefix => {
                formatter.write_str("frame length prefix is incomplete")
            }
            Self::FrameTooLarge { declared, maximum } => {
                write!(
                    formatter,
                    "frame declares {declared} bytes; maximum is {maximum}"
                )
            }
            Self::FrameLengthMismatch { declared, actual } => {
                write!(
                    formatter,
                    "frame declares {declared} bytes but contains {actual}"
                )
            }
            Self::MalformedJson(error) => write!(formatter, "malformed protocol JSON: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::MissingIdempotencyKey => {
                formatter.write_str("mutating request requires an idempotency key")
            }
            Self::InvalidLaunchToken => formatter.write_str("launch token must be 256-bit hex"),
            Self::InvalidResponseShape => {
                formatter.write_str("response must contain exactly one result or error")
            }
            Self::InvalidEventSequence => {
                formatter.write_str("event sequence must be greater than zero")
            }
        }
    }
}

impl Error for ProtocolError {}
