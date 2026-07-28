#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::fmt::{Display, Formatter};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use kesharon_ipc::{LocalEndpoint, connect};
use kesharon_protocol::{
    ClientRequest, HealthStatus, LaunchToken, MAX_FRAME_BYTES, ProtocolError, RequestMethod,
    ResponsePayload, ServerResponse, decode_server_response_frame, encode_frame,
};
use tauri::menu::MenuBuilder;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};

#[derive(Clone)]
pub struct DaemonClient {
    endpoint: LocalEndpoint,
    launch_token: String,
}

pub struct LaunchConfiguration {
    endpoint: LocalEndpoint,
    launch_token: String,
}

impl LaunchConfiguration {
    pub fn generate(_socket_directory: impl AsRef<Path>) -> Result<Self, HostError> {
        let mut token = [0_u8; 32];
        getrandom::fill(&mut token).map_err(|error| HostError::Random(error.to_string()))?;
        let mut launch_token = String::with_capacity(64);
        for byte in token {
            write!(&mut launch_token, "{byte:02x}")
                .expect("writing to an in-memory string cannot fail");
        }

        #[cfg(windows)]
        let endpoint_value = format!("kesharon-daemon-{}", uuid::Uuid::now_v7());

        #[cfg(unix)]
        let endpoint_value = _socket_directory
            .as_ref()
            .join(format!("kesharon-daemon-{}.sock", uuid::Uuid::now_v7()))
            .to_string_lossy()
            .into_owned();

        Ok(Self {
            endpoint: LocalEndpoint::new(endpoint_value)
                .map_err(|error| HostError::Endpoint(error.to_string()))?,
            launch_token,
        })
    }

    pub const fn endpoint(&self) -> &LocalEndpoint {
        &self.endpoint
    }

    pub fn launch_token(&self) -> &str {
        &self.launch_token
    }

    fn into_client(self) -> Result<DaemonClient, HostError> {
        DaemonClient::new(self.endpoint, self.launch_token)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitDecision {
    Restart,
    MarkRecoverable,
}

#[derive(Default)]
pub struct RestartBudget {
    restart_used: bool,
}

#[derive(Default)]
struct DaemonState {
    client: RwLock<Option<DaemonClient>>,
    child: Mutex<Option<CommandChild>>,
    restart_budget: Mutex<RestartBudget>,
}

impl RestartBudget {
    pub const fn record_unexpected_exit(&mut self) -> ExitDecision {
        if self.restart_used {
            ExitDecision::MarkRecoverable
        } else {
            self.restart_used = true;
            ExitDecision::Restart
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleAction {
    HideToTray,
    Exit,
}

pub const fn lifecycle_action(explicit_quit: bool) -> LifecycleAction {
    if explicit_quit {
        LifecycleAction::Exit
    } else {
        LifecycleAction::HideToTray
    }
}

impl DaemonClient {
    pub fn new(endpoint: LocalEndpoint, launch_token: String) -> Result<Self, HostError> {
        LaunchToken::parse_hex(&launch_token)?;
        Ok(Self {
            endpoint,
            launch_token,
        })
    }

    pub fn health(&self) -> Result<HealthSnapshot, HostError> {
        let response = self.request_health()?;
        match response.result() {
            Some(ResponsePayload::Health {
                status,
                protocol_version,
            }) => Ok(HealthSnapshot {
                status: *status,
                protocol_version: *protocol_version,
            }),
            _ => Err(HostError::UnexpectedResponse),
        }
    }

    pub fn request_health(&self) -> Result<ServerResponse, HostError> {
        let request = ClientRequest::new(
            uuid::Uuid::now_v7().to_string(),
            RequestMethod::Health,
            None,
        )?;
        let frame = encode_frame(&request)?;
        let mut stream = connect(&self.endpoint)?;
        stream.write_all(self.launch_token.as_bytes())?;
        stream.write_all(&frame)?;
        stream.flush()?;

        let mut length_prefix = [0_u8; 4];
        stream.read_exact(&mut length_prefix)?;
        let declared = u32::from_be_bytes(length_prefix) as usize;
        if declared > MAX_FRAME_BYTES {
            return Err(HostError::Protocol(ProtocolError::FrameTooLarge {
                declared,
                maximum: MAX_FRAME_BYTES,
            }));
        }

        let mut response_frame = Vec::with_capacity(declared + 4);
        response_frame.extend_from_slice(&length_prefix);
        response_frame.resize(declared + 4, 0);
        stream.read_exact(&mut response_frame[4..])?;
        Ok(decode_server_response_frame(&response_frame)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub status: HealthStatus,
    pub protocol_version: u16,
}

#[derive(Debug)]
pub enum HostError {
    Io(std::io::Error),
    Protocol(ProtocolError),
    Endpoint(String),
    Random(String),
    Runtime(String),
    StateUnavailable,
    UnexpectedResponse,
}

impl Display for HostError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "local daemon connection failed: {error}"),
            Self::Protocol(error) => write!(formatter, "daemon protocol failed: {error}"),
            Self::Endpoint(error) => write!(formatter, "local endpoint is invalid: {error}"),
            Self::Random(error) => write!(formatter, "secure random generation failed: {error}"),
            Self::Runtime(error) => write!(formatter, "desktop runtime failed: {error}"),
            Self::StateUnavailable => formatter.write_str("daemon state is unavailable"),
            Self::UnexpectedResponse => {
                formatter.write_str("daemon returned an unexpected response")
            }
        }
    }
}

impl Error for HostError {}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ProtocolError> for HostError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[tauri::command]
async fn daemon_health(state: State<'_, DaemonState>) -> Result<ServerResponse, String> {
    for _ in 0..25 {
        let client = state
            .client
            .read()
            .map_err(|_| HostError::StateUnavailable.to_string())?
            .clone()
            .ok_or_else(|| HostError::StateUnavailable.to_string())?;
        match client.request_health() {
            Ok(response) => return Ok(response),
            Err(_) => std::thread::sleep(Duration::from_millis(40)),
        }
    }

    Err("daemon did not become ready before the connection deadline".into())
}

fn start_daemon(app: &AppHandle, state: &DaemonState) -> Result<(), HostError> {
    let socket_directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| HostError::Runtime(error.to_string()))?
        .join("ipc");
    std::fs::create_dir_all(&socket_directory)?;

    let launch = LaunchConfiguration::generate(socket_directory)?;
    let endpoint_argument = launch.endpoint().as_str().to_owned();
    let stdin_token = format!("{}\n", launch.launch_token());
    let client = launch.into_client()?;
    let (mut receiver, mut child) = app
        .shell()
        .sidecar("kesharon-daemon")
        .map_err(|error| HostError::Runtime(error.to_string()))?
        .args(["--endpoint", &endpoint_argument])
        .spawn()
        .map_err(|error| HostError::Runtime(error.to_string()))?;
    child
        .write(stdin_token.as_bytes())
        .map_err(|error| HostError::Runtime(error.to_string()))?;

    *state
        .client
        .write()
        .map_err(|_| HostError::StateUnavailable)? = Some(client);
    *state
        .child
        .lock()
        .map_err(|_| HostError::StateUnavailable)? = Some(child);

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = receiver.recv().await {
            if matches!(event, CommandEvent::Terminated(_) | CommandEvent::Error(_)) {
                let state = app_handle.state::<DaemonState>();
                if let Ok(mut client) = state.client.write() {
                    client.take();
                }
                if let Ok(mut child) = state.child.lock() {
                    child.take();
                }
                let decision = state
                    .restart_budget
                    .lock()
                    .map_or(ExitDecision::MarkRecoverable, |mut budget| {
                        budget.record_unexpected_exit()
                    });
                if decision == ExitDecision::Restart {
                    let _ = start_daemon(&app_handle, state.inner());
                }
                break;
            }
        }
    });

    Ok(())
}

/// Runs the native desktop host and blocks until application exit.
///
/// # Panics
///
/// Panics when the embedded Tauri configuration cannot build the application.
pub fn run() {
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(DaemonState::default())
        .setup(|app| {
            let state = app.state::<DaemonState>();
            start_daemon(app.handle(), state.inner())?;

            let tray_menu = MenuBuilder::new(app)
                .text("show", "Show Kesharon")
                .text("quit", "Quit Kesharon")
                .build()?;
            let mut tray = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Kesharon local agent");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && lifecycle_action(false) == LifecycleAction::HideToTray
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![daemon_health])
        .build(tauri::generate_context!())
        .expect("Kesharon desktop host must build");

    application.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit)
            && let Ok(mut child) = app.state::<DaemonState>().child.lock()
            && let Some(child) = child.take()
        {
            let _ = child.kill();
        }
    });
}
