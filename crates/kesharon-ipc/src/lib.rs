#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io;

#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{Listener, ListenerOptions, Stream, prelude::*};

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

        let listener = ListenerOptions::new()
            .name(name)
            .try_overwrite(true)
            .create_sync()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(endpoint.as_str(), std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(Self { listener })
    }

    pub fn accept(&self) -> io::Result<Stream> {
        self.listener.accept()
    }
}

pub fn connect(endpoint: &LocalEndpoint) -> io::Result<Stream> {
    #[cfg(windows)]
    let name = endpoint.as_str().to_ns_name::<GenericNamespaced>()?;

    #[cfg(unix)]
    let name = std::path::Path::new(endpoint.as_str()).to_fs_name::<GenericFilePath>()?;

    Stream::connect(name)
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
