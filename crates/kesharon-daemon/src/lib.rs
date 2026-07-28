#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::{Read, Write};

use kesharon_ipc::LocalServer;
use kesharon_protocol::{
    HealthStatus, LaunchToken, MAX_FRAME_BYTES, PROTOCOL_VERSION, RequestMethod, ResponsePayload,
    ServerResponse, decode_client_request_frame, encode_frame,
};

pub struct Daemon {
    launch_token: LaunchToken,
}

impl Daemon {
    pub const fn new(launch_token: LaunchToken) -> Self {
        Self { launch_token }
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
        let response = match request.method() {
            RequestMethod::Health => ServerResponse::success(
                request.request_id(),
                ResponsePayload::Health {
                    status: HealthStatus::Ready,
                    protocol_version: PROTOCOL_VERSION,
                },
            ),
            RequestMethod::OpenProject { .. } => return Err(DaemonError::UnsupportedRequest),
        };

        encode_frame(&response).map_err(|error| DaemonError::Protocol(error.to_string()))
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
        if declared > MAX_FRAME_BYTES {
            return Err(DaemonError::FrameTooLarge {
                declared,
                maximum: MAX_FRAME_BYTES,
            });
        }

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
        let stream = server
            .accept()
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        let mut reader = &stream;
        let mut writer = &stream;
        let mut token = [0_u8; 64];
        reader
            .read_exact(&mut token)
            .map_err(|error| DaemonError::Io(error.to_string()))?;
        let token = std::str::from_utf8(&token).map_err(|_| DaemonError::AuthenticationFailed)?;
        self.serve_connection(token, &mut reader, &mut writer)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonError {
    AuthenticationFailed,
    FrameTooLarge { declared: usize, maximum: usize },
    Io(String),
    Protocol(String),
    UnsupportedRequest,
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
            Self::UnsupportedRequest => formatter.write_str("request is not implemented"),
        }
    }
}

impl Error for DaemonError {}
