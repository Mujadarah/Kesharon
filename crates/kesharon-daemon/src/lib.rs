#![forbid(unsafe_code)]

mod repository;
mod runtime;
mod session;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use kesharon_ipc::{LocalServer, LocalStream, read_exact_with_timeout, write_all_with_timeout};
use kesharon_protocol::{
    ClientRequest, ErrorCode, HealthStatus, LaunchToken, MAX_FRAME_BYTES, PROTOCOL_VERSION,
    RequestMethod, ResponsePayload, ServerResponse, StreamMessage, decode_client_request_frame,
    encode_frame,
};

pub use runtime::{MAX_ACTIVE_CONNECTIONS, ServerRuntime};
pub use session::{DaemonSession, SessionError, SessionSubscription};

#[derive(Clone)]
pub struct Daemon {
    launch_token: LaunchToken,
    session: Arc<DaemonSession>,
}

impl Daemon {
    pub fn new(launch_token: LaunchToken) -> Self {
        Self {
            launch_token,
            session: Arc::new(DaemonSession::production()),
        }
    }

    pub fn with_session(launch_token: LaunchToken, session: Arc<DaemonSession>) -> Self {
        Self {
            launch_token,
            session,
        }
    }

    pub fn handle_authenticated_frame(
        &self,
        presented_token: &str,
        frame: &[u8],
    ) -> Result<Vec<u8>, DaemonError> {
        if !self.launch_token.matches_hex(presented_token) {
            return Err(DaemonError::AuthenticationFailed);
        }
        let request = decode_client_request_frame(frame)
            .map_err(|error| DaemonError::Protocol(error.to_string()))?;
        let response = self.dispatch_request(&request);
        encode_frame(&response).map_err(|error| DaemonError::Protocol(error.to_string()))
    }

    fn dispatch_request(&self, request: &ClientRequest) -> ServerResponse {
        match request.method() {
            RequestMethod::Health => ServerResponse::success(
                request.request_id(),
                ResponsePayload::Health {
                    status: HealthStatus::Ready,
                    protocol_version: PROTOCOL_VERSION,
                },
            ),
            RequestMethod::SubscribeEvents => ServerResponse::success(
                request.request_id(),
                ResponsePayload::SubscriptionReady {
                    stream_id: self.session.stream_id(),
                },
            ),
            RequestMethod::OpenProject { .. } | RequestMethod::CancelRequest { .. } => {
                self.session.dispatch(request)
            }
        }
    }

    pub fn serve_connection<R: Read, W: Write>(
        &self,
        presented_token: &str,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<(), DaemonError> {
        let mut prefix = [0_u8; 4];
        reader
            .read_exact(&mut prefix)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        let declared = u32::from_be_bytes(prefix) as usize;
        validate_frame_size(declared)?;
        let mut frame = Vec::with_capacity(declared + 4);
        frame.extend_from_slice(&prefix);
        frame.resize(declared + 4, 0);
        reader
            .read_exact(&mut frame[4..])
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        let response = self.handle_authenticated_frame(presented_token, &frame)?;
        writer
            .write_all(&response)
            .and_then(|()| writer.flush())
            .map_err(|error| DaemonError::Io(error.to_string()))
    }

    pub fn serve_local_connection(&self, server: &LocalServer) -> Result<(), DaemonError> {
        self.serve_local_connection_with_timeout(server, Duration::from_secs(5))
    }

    pub fn serve_local_connection_with_timeout(
        &self,
        server: &LocalServer,
        timeout: Duration,
    ) -> Result<(), DaemonError> {
        let stream = server
            .accept_with_timeout(timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        self.serve_local_stream(&stream, timeout)
    }

    fn read_authenticated_request(
        &self,
        stream: &LocalStream,
        timeout: Duration,
    ) -> Result<ClientRequest, DaemonError> {
        let mut token = [0_u8; 64];
        read_exact_with_timeout(stream, &mut token, timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        let token = std::str::from_utf8(&token).map_err(|_| DaemonError::AuthenticationFailed)?;
        let mut prefix = [0_u8; 4];
        read_exact_with_timeout(stream, &mut prefix, timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        let declared = u32::from_be_bytes(prefix) as usize;
        validate_frame_size(declared)?;
        let mut frame = Vec::with_capacity(declared + 4);
        frame.extend_from_slice(&prefix);
        frame.resize(declared + 4, 0);
        read_exact_with_timeout(stream, &mut frame[4..], timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        if !self.launch_token.matches_hex(token) {
            return Err(DaemonError::AuthenticationFailed);
        }
        decode_client_request_frame(&frame)
            .map_err(|error| DaemonError::Protocol(error.to_string()))
    }

    fn serve_local_stream(
        &self,
        stream: &LocalStream,
        timeout: Duration,
    ) -> Result<(), DaemonError> {
        let request = self.read_authenticated_request(stream, timeout)?;
        if matches!(request.method(), RequestMethod::SubscribeEvents) {
            return self.serve_subscription(stream, timeout, &request);
        }
        let response = encode_frame(&self.dispatch_request(&request))
            .map_err(|error| DaemonError::Protocol(error.to_string()))?;
        write_all_with_timeout(stream, &response, timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))
    }

    fn serve_subscription(
        &self,
        stream: &LocalStream,
        timeout: Duration,
        request: &ClientRequest,
    ) -> Result<(), DaemonError> {
        let subscription = self
            .session
            .subscribe()
            .map_err(|error| DaemonError::Session(format!("{error:?}")))?;
        let ready = encode_frame(&self.dispatch_request(request))
            .map_err(|error| DaemonError::Protocol(error.to_string()))?;
        let snapshot = encode_frame(&StreamMessage::Snapshot {
            snapshot: subscription.snapshot,
        })
        .map_err(|error| DaemonError::Protocol(error.to_string()))?;
        write_all_with_timeout(stream, &ready, timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        write_all_with_timeout(stream, &snapshot, timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        while let Ok(message) = subscription.receiver.recv() {
            let frame =
                encode_frame(&message).map_err(|error| DaemonError::Protocol(error.to_string()))?;
            write_all_with_timeout(stream, &frame, timeout)
                .map_err(|error| DaemonError::Io(error.to_string()))?;
        }
        Ok(())
    }

    fn reject_busy_connection(
        &self,
        stream: &LocalStream,
        timeout: Duration,
    ) -> Result<(), DaemonError> {
        let request = self.read_authenticated_request(stream, timeout)?;
        let response = ServerResponse::failure(
            request.request_id(),
            ErrorCode::ServerBusy,
            "The daemon has reached its concurrent connection limit",
        );
        let frame =
            encode_frame(&response).map_err(|error| DaemonError::Protocol(error.to_string()))?;
        write_all_with_timeout(stream, &frame, timeout)
            .map_err(|error| DaemonError::Io(error.to_string()))
    }
}

fn validate_frame_size(declared: usize) -> Result<(), DaemonError> {
    if declared > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge {
            declared,
            maximum: MAX_FRAME_BYTES,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonError {
    AuthenticationFailed,
    FrameTooLarge { declared: usize, maximum: usize },
    Io(String),
    Protocol(String),
    Session(String),
}

impl Display for DaemonError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthenticationFailed => formatter.write_str("launch authentication failed"),
            Self::FrameTooLarge { declared, maximum } => {
                write!(
                    formatter,
                    "frame declares {declared} bytes; maximum is {maximum}"
                )
            }
            Self::Io(message) => write!(formatter, "I/O error: {message}"),
            Self::Protocol(message) => write!(formatter, "protocol error: {message}"),
            Self::Session(message) => write!(formatter, "session error: {message}"),
        }
    }
}

impl Error for DaemonError {}
