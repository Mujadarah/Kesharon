#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use kesharon_ipc::{
    LocalEndpoint, LocalStream, connect_with_timeout, read_exact_with_timeout,
    write_all_with_timeout,
};
use kesharon_protocol::{
    ClientRequest, DaemonEvent, HealthStatus, LaunchToken, MAX_FRAME_BYTES, ProtocolError,
    RequestMethod, ResponsePayload, ServerResponse, StreamMessage, WorkspaceSnapshot,
    decode_server_response_frame, decode_stream_message_frame, encode_frame,
};
use tauri::ipc::Channel;
use tauri::menu::MenuBuilder;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};

pub const INITIAL_BACKOFF: Duration = Duration::from_millis(50);
pub const MAX_BACKOFF: Duration = Duration::from_secs(1);

#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub const fn next_backoff(current: Duration) -> Duration {
    let doubled = current.as_millis().saturating_mul(2);
    if doubled > MAX_BACKOFF.as_millis() {
        MAX_BACKOFF
    } else {
        Duration::from_millis(doubled as u64)
    }
}

#[derive(Clone)]
pub struct DaemonClient {
    endpoint: LocalEndpoint,
    launch_token: String,
    io_timeout: Duration,
}

pub struct LaunchConfiguration {
    endpoint: LocalEndpoint,
    launch_token: String,
}

impl LaunchConfiguration {
    #[cfg_attr(windows, allow(unused_variables))]
    pub fn generate(socket_directory: impl AsRef<Path>) -> Result<Self, HostError> {
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
        let endpoint_value = socket_directory
            .as_ref()
            .join(format!("k-{}.sock", uuid::Uuid::now_v7().simple()))
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
pub struct DaemonState {
    pub client: Arc<RwLock<Option<DaemonClient>>>,
    pub child: Mutex<Option<CommandChild>>,
    pub restart_budget: Mutex<RestartBudget>,
    pub active_subscription_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl DaemonState {
    pub fn new_with_client(client: DaemonClient) -> Self {
        Self {
            client: Arc::new(RwLock::new(Some(client))),
            child: Mutex::new(None),
            restart_budget: Mutex::new(RestartBudget::default()),
            active_subscription_cancel: Mutex::new(None),
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessObservation {
    Output,
    StreamError,
    Terminated,
}

pub const fn should_replace_daemon(observation: ProcessObservation) -> bool {
    matches!(observation, ProcessObservation::Terminated)
}

pub const fn lifecycle_action(explicit_quit: bool) -> LifecycleAction {
    if explicit_quit {
        LifecycleAction::Exit
    } else {
        LifecycleAction::HideToTray
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DaemonStreamUpdate {
    Message { message: StreamMessage },
    Reconnecting,
    Unavailable { message: String },
}

pub struct DaemonSubscription {
    stream: LocalStream,
    stream_id: String,
    initial_snapshot: WorkspaceSnapshot,
    #[allow(dead_code)]
    io_timeout: Duration,
}

impl DaemonSubscription {
    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub fn initial_snapshot(&self) -> &WorkspaceSnapshot {
        &self.initial_snapshot
    }

    pub fn read_next_message(&self, timeout: Duration) -> Result<StreamMessage, HostError> {
        let mut prefix = [0_u8; 4];
        read_exact_with_timeout(&self.stream, &mut prefix, timeout)?;
        let declared = u32::from_be_bytes(prefix) as usize;
        if declared > MAX_FRAME_BYTES {
            return Err(HostError::Protocol(ProtocolError::FrameTooLarge {
                declared,
                maximum: MAX_FRAME_BYTES,
            }));
        }

        let mut frame = Vec::with_capacity(declared + 4);
        frame.extend_from_slice(&prefix);
        frame.resize(declared + 4, 0);
        read_exact_with_timeout(&self.stream, &mut frame[4..], timeout)?;
        Ok(decode_stream_message_frame(&frame)?)
    }

    pub fn read_next_event(&self, timeout: Duration) -> Result<DaemonEvent, HostError> {
        match self.read_next_message(timeout)? {
            StreamMessage::Event { event } => Ok(event),
            StreamMessage::Snapshot { .. } => Err(HostError::UnexpectedResponse),
        }
    }
}

impl DaemonClient {
    pub fn new(endpoint: LocalEndpoint, launch_token: String) -> Result<Self, HostError> {
        Self::new_with_timeout(endpoint, launch_token, Duration::from_secs(2))
    }

    pub fn new_with_timeout(
        endpoint: LocalEndpoint,
        launch_token: String,
        io_timeout: Duration,
    ) -> Result<Self, HostError> {
        LaunchToken::parse_hex(&launch_token)?;
        if io_timeout.is_zero() {
            return Err(HostError::InvalidTimeout);
        }
        Ok(Self {
            endpoint,
            launch_token,
            io_timeout,
        })
    }

    pub fn send_request(&self, request: &ClientRequest) -> Result<ServerResponse, HostError> {
        let frame = encode_frame(request)?;
        let stream = connect_with_timeout(&self.endpoint, self.io_timeout)?;
        write_all_with_timeout(&stream, self.launch_token.as_bytes(), self.io_timeout)?;
        write_all_with_timeout(&stream, &frame, self.io_timeout)?;

        let mut length_prefix = [0_u8; 4];
        read_exact_with_timeout(&stream, &mut length_prefix, self.io_timeout)?;
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
        read_exact_with_timeout(&stream, &mut response_frame[4..], self.io_timeout)?;
        Ok(decode_server_response_frame(&response_frame)?)
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
        self.send_request(&request)
    }

    pub fn open_project(
        &self,
        path: &str,
        request_id: &str,
        idempotency_key: &str,
    ) -> Result<ServerResponse, HostError> {
        let request = ClientRequest::new(
            request_id,
            RequestMethod::OpenProject {
                path: path.to_owned(),
            },
            Some(idempotency_key.to_owned()),
        )?;
        self.send_request(&request)
    }

    pub fn cancel_request(
        &self,
        target_request_id: &str,
        request_id: &str,
        idempotency_key: &str,
    ) -> Result<ServerResponse, HostError> {
        let request = ClientRequest::new(
            request_id,
            RequestMethod::CancelRequest {
                target_request_id: target_request_id.to_owned(),
            },
            Some(idempotency_key.to_owned()),
        )?;
        self.send_request(&request)
    }

    pub fn subscribe(&self) -> Result<DaemonSubscription, HostError> {
        let request = ClientRequest::new(
            uuid::Uuid::now_v7().to_string(),
            RequestMethod::SubscribeEvents,
            None,
        )?;
        let frame = encode_frame(&request)?;
        let stream = connect_with_timeout(&self.endpoint, self.io_timeout)?;
        write_all_with_timeout(&stream, self.launch_token.as_bytes(), self.io_timeout)?;
        write_all_with_timeout(&stream, &frame, self.io_timeout)?;

        // 1. Read ServerResponse (SubscriptionReady)
        let mut prefix = [0_u8; 4];
        read_exact_with_timeout(&stream, &mut prefix, self.io_timeout)?;
        let declared = u32::from_be_bytes(prefix) as usize;
        if declared > MAX_FRAME_BYTES {
            return Err(HostError::Protocol(ProtocolError::FrameTooLarge {
                declared,
                maximum: MAX_FRAME_BYTES,
            }));
        }
        let mut ready_frame = Vec::with_capacity(declared + 4);
        ready_frame.extend_from_slice(&prefix);
        ready_frame.resize(declared + 4, 0);
        read_exact_with_timeout(&stream, &mut ready_frame[4..], self.io_timeout)?;
        let response = decode_server_response_frame(&ready_frame)?;
        let stream_id = match response.result() {
            Some(ResponsePayload::SubscriptionReady { stream_id }) => stream_id.clone(),
            _ => return Err(HostError::UnexpectedResponse),
        };

        // 2. Read initial StreamMessage (Snapshot)
        read_exact_with_timeout(&stream, &mut prefix, self.io_timeout)?;
        let declared = u32::from_be_bytes(prefix) as usize;
        if declared > MAX_FRAME_BYTES {
            return Err(HostError::Protocol(ProtocolError::FrameTooLarge {
                declared,
                maximum: MAX_FRAME_BYTES,
            }));
        }
        let mut snapshot_frame = Vec::with_capacity(declared + 4);
        snapshot_frame.extend_from_slice(&prefix);
        snapshot_frame.resize(declared + 4, 0);
        read_exact_with_timeout(&stream, &mut snapshot_frame[4..], self.io_timeout)?;
        let initial_message = decode_stream_message_frame(&snapshot_frame)?;
        let initial_snapshot = match initial_message {
            StreamMessage::Snapshot { snapshot } => snapshot,
            StreamMessage::Event { .. } => return Err(HostError::UnexpectedResponse),
        };

        Ok(DaemonSubscription {
            stream,
            stream_id,
            initial_snapshot,
            io_timeout: self.io_timeout,
        })
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
    InvalidTimeout,
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
            Self::InvalidTimeout => formatter.write_str("daemon I/O timeout must be positive"),
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

pub mod commands {
    use super::{
        AppHandle, Arc, AtomicBool, Channel, DaemonState, DaemonStreamUpdate, DialogExt, Duration,
        HostError, INITIAL_BACKOFF, Mutex, Ordering, ServerResponse, State, StreamMessage,
        next_backoff,
    };

    pub async fn execute_choose_project_directory(
        app: &AppHandle,
    ) -> Result<Option<String>, String> {
        let app_handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let folder = app_handle.dialog().file().blocking_pick_folder();
            Ok(folder.map(|path| path.to_string()))
        })
        .await
        .map_err(|error| error.to_string())?
    }

    pub async fn execute_open_project(
        state: &DaemonState,
        path: String,
        request_id: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<ServerResponse, String> {
        let client = state
            .client
            .read()
            .map_err(|_| HostError::StateUnavailable.to_string())?
            .clone()
            .ok_or_else(|| HostError::StateUnavailable.to_string())?;

        let req_id = request_id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        let idemp_key = idempotency_key.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

        tauri::async_runtime::spawn_blocking(move || {
            client
                .open_project(&path, &req_id, &idemp_key)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    pub async fn execute_cancel_request(
        state: &DaemonState,
        target_request_id: String,
        request_id: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<ServerResponse, String> {
        let client = state
            .client
            .read()
            .map_err(|_| HostError::StateUnavailable.to_string())?
            .clone()
            .ok_or_else(|| HostError::StateUnavailable.to_string())?;

        let req_id = request_id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        let idemp_key = idempotency_key.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

        tauri::async_runtime::spawn_blocking(move || {
            client
                .cancel_request(&target_request_id, &req_id, &idemp_key)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    #[allow(clippy::too_many_lines)]
    pub fn execute_subscribe_daemon_events(
        state: &DaemonState,
        on_event: Channel<DaemonStreamUpdate>,
    ) -> Result<(), String> {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut active_cancel) = state.active_subscription_cancel.lock() {
            if let Some(prev) = active_cancel.take() {
                prev.store(true, Ordering::Release);
            }
            *active_cancel = Some(Arc::clone(&cancel_flag));
        }

        let client_handle = Arc::clone(&state.client);

        tauri::async_runtime::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;
            let mut has_connected_once = false;
            let mut is_reconnecting = false;

            while !cancel_flag.load(Ordering::Acquire) {
                let client_opt = {
                    if let Ok(guard) = client_handle.read() {
                        guard.clone()
                    } else {
                        None
                    }
                };

                let Some(client) = client_opt else {
                    if has_connected_once && !is_reconnecting {
                        let _ = on_event.send(DaemonStreamUpdate::Reconnecting);
                        is_reconnecting = true;
                    }
                    if cancel_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let current_delay = backoff;
                    let _ = tauri::async_runtime::spawn_blocking(move || {
                        std::thread::sleep(current_delay);
                    })
                    .await;
                    backoff = next_backoff(backoff);
                    continue;
                };

                let cancel_check = Arc::clone(&cancel_flag);
                let sub_res = tauri::async_runtime::spawn_blocking(move || {
                    if cancel_check.load(Ordering::Acquire) {
                        return Err(HostError::Runtime("cancelled".into()));
                    }
                    client.subscribe()
                })
                .await;

                if cancel_flag.load(Ordering::Acquire) {
                    break;
                }

                let Ok(Ok(subscription)) = sub_res else {
                    if has_connected_once && !is_reconnecting {
                        let _ = on_event.send(DaemonStreamUpdate::Reconnecting);
                        is_reconnecting = true;
                    }
                    if cancel_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let current_delay = backoff;
                    let _ = tauri::async_runtime::spawn_blocking(move || {
                        std::thread::sleep(current_delay);
                    })
                    .await;
                    backoff = next_backoff(backoff);
                    continue;
                };

                backoff = INITIAL_BACKOFF;
                has_connected_once = true;
                is_reconnecting = false;

                let initial_snapshot = subscription.initial_snapshot().clone();
                let mut current_stream_id = subscription.stream_id().to_string();
                let mut expected_sequence = initial_snapshot.last_sequence() + 1;

                if on_event
                    .send(DaemonStreamUpdate::Message {
                        message: StreamMessage::Snapshot {
                            snapshot: initial_snapshot,
                        },
                    })
                    .is_err()
                {
                    break;
                }

                let subscription = Arc::new(Mutex::new(subscription));

                while !cancel_flag.load(Ordering::Acquire) {
                    let sub_clone = Arc::clone(&subscription);
                    let cancel_clone = Arc::clone(&cancel_flag);

                    let read_res = tauri::async_runtime::spawn_blocking(move || {
                        if cancel_clone.load(Ordering::Acquire) {
                            return Err(HostError::Runtime("cancelled".into()));
                        }
                        sub_clone
                            .lock()
                            .map_err(|_| HostError::Runtime("mutex poisoned".into()))?
                            .read_next_message(Duration::from_millis(150))
                    })
                    .await;

                    if cancel_flag.load(Ordering::Acquire) {
                        break;
                    }

                    match read_res {
                        Ok(Ok(message)) => match &message {
                            StreamMessage::Event { event } => {
                                if event.stream_id() != current_stream_id
                                    || event.sequence() != expected_sequence
                                {
                                    let _ = on_event.send(DaemonStreamUpdate::Reconnecting);
                                    is_reconnecting = true;
                                    break;
                                }
                                expected_sequence = event.sequence() + 1;
                                if on_event
                                    .send(DaemonStreamUpdate::Message { message })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            StreamMessage::Snapshot { snapshot } => {
                                current_stream_id = snapshot.stream_id().to_string();
                                expected_sequence = snapshot.last_sequence() + 1;
                                if on_event
                                    .send(DaemonStreamUpdate::Message { message })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        },
                        Ok(Err(HostError::Io(err)))
                            if err.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            let client_to_check = {
                                if let Ok(guard) = client_handle.read() {
                                    guard.clone()
                                } else {
                                    None
                                }
                            };
                            let is_healthy = match client_to_check {
                                Some(client) => tauri::async_runtime::spawn_blocking(move || {
                                    client.health().is_ok()
                                })
                                .await
                                .unwrap_or(false),
                                None => false,
                            };
                            if !is_healthy {
                                let _ = on_event.send(DaemonStreamUpdate::Reconnecting);
                                is_reconnecting = true;
                                break;
                            }
                        }
                        Ok(Err(_)) | Err(_) => {
                            let _ = on_event.send(DaemonStreamUpdate::Reconnecting);
                            is_reconnecting = true;
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn execute_health(state: &DaemonState) -> Result<ServerResponse, String> {
        let client = state
            .client
            .read()
            .map_err(|_| HostError::StateUnavailable.to_string())?
            .clone()
            .ok_or_else(|| HostError::StateUnavailable.to_string())?;

        tauri::async_runtime::spawn_blocking(move || {
            for _ in 0..25 {
                match client.request_health() {
                    Ok(response) => return Ok(response),
                    Err(_) => std::thread::sleep(Duration::from_millis(40)),
                }
            }
            Err("daemon did not become ready before the connection deadline".into())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    #[tauri::command]
    pub async fn choose_project_directory(app: AppHandle) -> Result<Option<String>, String> {
        execute_choose_project_directory(&app).await
    }

    #[tauri::command]
    pub async fn daemon_open_project(
        path: String,
        request_id: Option<String>,
        idempotency_key: Option<String>,
        state: State<'_, DaemonState>,
    ) -> Result<ServerResponse, String> {
        execute_open_project(state.inner(), path, request_id, idempotency_key).await
    }

    #[tauri::command]
    pub async fn daemon_cancel_request(
        target_request_id: String,
        request_id: Option<String>,
        idempotency_key: Option<String>,
        state: State<'_, DaemonState>,
    ) -> Result<ServerResponse, String> {
        execute_cancel_request(
            state.inner(),
            target_request_id,
            request_id,
            idempotency_key,
        )
        .await
    }

    #[tauri::command]
    pub async fn subscribe_daemon_events(
        on_event: Channel<DaemonStreamUpdate>,
        state: State<'_, DaemonState>,
    ) -> Result<(), String> {
        execute_subscribe_daemon_events(state.inner(), on_event)
    }

    #[tauri::command]
    pub async fn daemon_health(state: State<'_, DaemonState>) -> Result<ServerResponse, String> {
        execute_health(state.inner()).await
    }
}

fn start_daemon(app: &AppHandle, state: &DaemonState) -> Result<(), HostError> {
    let socket_directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| HostError::Runtime(error.to_string()))?
        .join("ipc");
    std::fs::create_dir_all(&socket_directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&socket_directory, std::fs::Permissions::from_mode(0o700))?;
    }

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
    if let Err(error) = child.write(stdin_token.as_bytes()) {
        let _ = child.kill();
        return Err(HostError::Runtime(error.to_string()));
    }

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
            let observation = match event {
                CommandEvent::Terminated(_) => ProcessObservation::Terminated,
                CommandEvent::Error(_) => ProcessObservation::StreamError,
                _ => ProcessObservation::Output,
            };
            if should_replace_daemon(observation) {
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
        .plugin(tauri_plugin_dialog::init())
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
        .invoke_handler(tauri::generate_handler![
            commands::choose_project_directory,
            commands::daemon_open_project,
            commands::daemon_cancel_request,
            commands::subscribe_daemon_events,
            commands::daemon_health,
        ])
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
