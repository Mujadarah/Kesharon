#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use interprocess::ConnectWaitMode;
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{
    ConnectOptions, Listener, ListenerNonblockingMode, ListenerOptions, Stream, prelude::*,
};

pub type LocalStream = Stream;

/// Protected DACL for Windows named pipes granting full access to Owner Rights (`OW`, S-1-3-4)
/// and Local System (`SY`, S-1-5-18) while denying access to all other local user accounts.
#[cfg(windows)]
const WINDOWS_PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

#[cfg(windows)]
pub const fn windows_pipe_sddl() -> &'static str {
    WINDOWS_PIPE_SDDL
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalEndpoint(String);

impl LocalEndpoint {
    pub fn new(value: impl Into<String>) -> Result<Self, EndpointError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(EndpointError::Blank);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct LocalServer {
    listener: Listener,
}

impl LocalServer {
    pub fn bind(endpoint: &LocalEndpoint) -> io::Result<Self> {
        #[cfg(windows)]
        let name = endpoint.as_str().to_ns_name::<GenericNamespaced>()?;

        #[cfg(unix)]
        let name = std::path::Path::new(endpoint.as_str()).to_fs_name::<GenericFilePath>()?;

        #[cfg(windows)]
        let listener = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            use widestring::U16CString;

            let sddl = U16CString::from_str(WINDOWS_PIPE_SDDL)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let descriptor = SecurityDescriptor::deserialize(&sddl)?;
            ListenerOptions::new()
                .name(name)
                .try_overwrite(true)
                .security_descriptor(descriptor)
                .create_sync()?
        };

        #[cfg(unix)]
        let listener = {
            let options = ListenerOptions::new().name(name).try_overwrite(true);

            #[cfg(any(
                target_os = "android",
                target_os = "freebsd",
                target_os = "linux",
                target_os = "openbsd"
            ))]
            let options = {
                use interprocess::os::unix::local_socket::ListenerOptionsExt;

                options.mode(0o600)
            };

            let listener = options.create_sync()?;

            #[cfg(target_os = "macos")]
            {
                use std::os::unix::fs::PermissionsExt;

                std::fs::set_permissions(
                    endpoint.as_str(),
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }

            listener
        };

        Ok(Self { listener })
    }

    pub fn accept(&self) -> io::Result<Stream> {
        self.listener.accept()
    }

    pub fn accept_with_timeout(&self, timeout: Duration) -> io::Result<Stream> {
        if timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "I/O timeout must be positive",
            ));
        }
        self.listener
            .set_nonblocking(ListenerNonblockingMode::Accept)?;
        let deadline = Instant::now() + timeout;
        loop {
            match self.listener.accept() {
                Ok(stream) => {
                    stream.set_nonblocking(true)?;
                    return Ok(stream);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_for_io(deadline)?;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

pub fn connect(endpoint: &LocalEndpoint) -> io::Result<Stream> {
    #[cfg(windows)]
    let name = endpoint.as_str().to_ns_name::<GenericNamespaced>()?;

    #[cfg(unix)]
    let name = std::path::Path::new(endpoint.as_str()).to_fs_name::<GenericFilePath>()?;

    Stream::connect(name)
}

pub fn connect_with_timeout(endpoint: &LocalEndpoint, timeout: Duration) -> io::Result<Stream> {
    if timeout.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "I/O timeout must be positive",
        ));
    }
    #[cfg(windows)]
    let name = endpoint.as_str().to_ns_name::<GenericNamespaced>()?;

    #[cfg(unix)]
    let name = std::path::Path::new(endpoint.as_str()).to_fs_name::<GenericFilePath>()?;

    let stream = ConnectOptions::new()
        .name(name)
        .wait_mode(ConnectWaitMode::Timeout(timeout))
        .connect_sync()?;
    stream.set_nonblocking(true)?;
    Ok(stream)
}

pub fn set_stream_nonblocking(stream: &LocalStream, nonblocking: bool) -> io::Result<()> {
    stream.set_nonblocking(nonblocking)
}

pub fn read_exact_with_timeout(
    stream: &Stream,
    mut buffer: &mut [u8],
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut reader = stream;
    while !buffer.is_empty() {
        match reader.read(buffer) {
            Ok(0) => wait_for_io(deadline)?,
            Ok(read) => buffer = &mut buffer[read..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_for_io(deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn write_all_with_timeout(
    stream: &Stream,
    mut buffer: &[u8],
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut writer = stream;
    while !buffer.is_empty() {
        match writer.write(buffer) {
            Ok(0) => wait_for_io(deadline)?,
            Ok(written) => buffer = &buffer[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_for_io(deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn wait_for_io(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "local IPC operation exceeded its deadline",
        ));
    }
    std::thread::sleep(Duration::from_millis(1));
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EndpointError {
    Blank,
}

impl Display for EndpointError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blank => formatter.write_str("local IPC endpoint must not be blank"),
        }
    }
}

impl Error for EndpointError {}
