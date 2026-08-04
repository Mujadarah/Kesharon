use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kesharon_ipc::LocalServer;

use crate::{Daemon, DaemonError};

pub const MAX_ACTIVE_CONNECTIONS: usize = 8;

pub struct ServerRuntime {
    daemon: Daemon,
    active_connections: Arc<AtomicUsize>,
    io_timeout: Duration,
}

impl ServerRuntime {
    pub fn new(daemon: Daemon) -> Self {
        Self {
            daemon,
            active_connections: Arc::new(AtomicUsize::new(0)),
            io_timeout: Duration::from_secs(5),
        }
    }

    pub fn run(&self, server: &LocalServer) -> Result<(), DaemonError> {
        loop {
            let stream = match server.accept_with_timeout(Duration::from_millis(100)) {
                Ok(stream) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(error) => return Err(DaemonError::Io(error.to_string())),
            };
            if let Some(permit) = ConnectionPermit::acquire(Arc::clone(&self.active_connections)) {
                let daemon = self.daemon.clone();
                let timeout = self.io_timeout;
                if let Err(error) = std::thread::Builder::new()
                    .name("kesharon-connection".into())
                    .spawn(move || {
                        let _permit = permit;
                        if let Err(error) = daemon.serve_local_stream(&stream, timeout) {
                            eprintln!("kesharon-daemon: connection failed: {error}");
                        }
                    })
                {
                    eprintln!("kesharon-daemon: connection worker failed: {error}");
                }
            } else if let Err(error) = self.daemon.reject_busy_connection(&stream, self.io_timeout)
            {
                eprintln!("kesharon-daemon: busy connection failed: {error}");
            }
        }
    }
}

struct ConnectionPermit {
    active_connections: Arc<AtomicUsize>,
}

impl ConnectionPermit {
    fn acquire(active_connections: Arc<AtomicUsize>) -> Option<Self> {
        active_connections
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_CONNECTIONS).then_some(active + 1)
            })
            .ok()
            .map(|_| Self { active_connections })
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.active_connections.fetch_sub(1, Ordering::AcqRel);
    }
}
