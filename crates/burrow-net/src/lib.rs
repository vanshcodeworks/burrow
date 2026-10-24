//! Networking for Burrow: stable port allocation and, in M4, the loopback
//! reverse proxy.
//!
//! Ports are remembered per project so URLs stay stable across restarts. A
//! remembered port is only reused when it can still be bound on `127.0.0.1`;
//! this also avoids collisions with other projects running at the same time.
//!
//! The proxy and `*.localhost` routing are added in milestone M4.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};

use rand::Rng;
use serde::{Deserialize, Serialize};

/// Port range used for automatic allocation.
pub const AUTO_PORT_START: u16 = 40_000;
/// End of the automatic port range, exclusive.
pub const AUTO_PORT_END: u16 = 50_000;
const PORTS_FILE: &str = "ports.json";

/// A port allocation failure.
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    /// The assignment file could not be read or written.
    #[error("could not access {path}: {source}")]
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// A fixed port is already in use.
    #[error("port {port} is already in use; choose another port or use `port = \"auto\"`")]
    FixedPortBusy {
        /// The requested port.
        port: u16,
    },
    /// Automatic allocation could not find a free port.
    #[error("could not find a free port in {start}-{end}; free a port or set one explicitly")]
    Exhausted {
        /// First port in the scanned range.
        start: u16,
        /// Last port in the scanned range.
        end: u16,
    },
    /// The assignment file is malformed.
    #[error("invalid port assignment file {path}: {message}")]
    Invalid {
        /// File that failed.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PortFile {
    version: u32,
    #[serde(default)]
    ports: BTreeMap<String, u16>,
}

/// Remembers per-service ports under `.burrow/ports.json`.
#[derive(Debug)]
pub struct PortAllocator {
    path: PathBuf,
    ports: BTreeMap<String, u16>,
    claimed: BTreeSet<u16>,
}

impl PortAllocator {
    /// Loads assignments from `state_dir`, or starts empty when no file exists.
    pub fn load(state_dir: &Path) -> Result<Self, PortError> {
        let path = state_dir.join(PORTS_FILE);
        let file = match std::fs::read_to_string(&path) {
            Ok(contents) => {
                serde_json::from_str::<PortFile>(&contents).map_err(|error| PortError::Invalid {
                    path: path.clone(),
                    message: error.to_string(),
                })?
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => PortFile {
                version: 1,
                ..PortFile::default()
            },
            Err(source) => {
                return Err(PortError::Io {
                    path: path.clone(),
                    source,
                })
            }
        };

        Ok(Self {
            path,
            ports: file.ports,
            claimed: BTreeSet::new(),
        })
    }

    /// Allocates a port for `service`, reusing the remembered assignment when
    /// it is still free.
    ///
    /// A fixed request must be free; it is never silently replaced.
    pub fn allocate(&mut self, service: &str, fixed: Option<u16>) -> Result<u16, PortError> {
        if let Some(port) = fixed {
            if self.ports.get(service) == Some(&port) || is_free(port) {
                self.ports.insert(service.to_owned(), port);
                self.claimed.insert(port);
                return Ok(port);
            }
            return Err(PortError::FixedPortBusy { port });
        }

        if let Some(port) = self.ports.get(service).copied() {
            if is_free(port) && self.claimed.insert(port) {
                return Ok(port);
            }
            self.ports.remove(service);
        }

        let mut rng = rand::rng();
        for _ in 0..64 {
            let port = rng.random_range(AUTO_PORT_START..AUTO_PORT_END);
            if !self.claimed.contains(&port) && is_free(port) {
                self.ports.insert(service.to_owned(), port);
                self.claimed.insert(port);
                return Ok(port);
            }
        }

        // Deterministic fallback: scan once rather than failing after a streak
        // of random collisions.
        for port in AUTO_PORT_START..AUTO_PORT_END {
            if !self.claimed.contains(&port) && is_free(port) {
                self.ports.insert(service.to_owned(), port);
                self.claimed.insert(port);
                return Ok(port);
            }
        }

        Err(PortError::Exhausted {
            start: AUTO_PORT_START,
            end: AUTO_PORT_END - 1,
        })
    }

    /// Reserves a port already owned by another listener (for example the
    /// control socket) so it is not handed to a service.
    pub fn reserve(&mut self, port: u16) {
        self.claimed.insert(port);
    }

    /// Forgets one service's assignment.
    pub fn release(&mut self, service: &str) {
        if let Some(port) = self.ports.remove(service) {
            self.claimed.remove(&port);
        }
    }

    /// Returns the remembered port for a service, if any.
    pub fn remembered(&self, service: &str) -> Option<u16> {
        self.ports.get(service).copied()
    }

    /// Writes assignments atomically.
    pub fn save(&self) -> Result<(), PortError> {
        let file = PortFile {
            version: 1,
            ports: self.ports.clone(),
        };
        let encoded = serde_json::to_vec_pretty(&file).expect("port assignments are serializable");
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        if let Err(source) = std::fs::create_dir_all(parent) {
            return Err(PortError::Io {
                path: parent.to_path_buf(),
                source,
            });
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, encoded).map_err(|source| PortError::Io {
            path: temporary.clone(),
            source,
        })?;
        std::fs::rename(&temporary, &self.path).map_err(|source| PortError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

/// Returns true when `127.0.0.1:port` can be bound right now.
pub fn is_free(port: u16) -> bool {
    TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{is_free, PortAllocator};
    use std::net::TcpListener;

    fn free_port() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral");
        listener.local_addr().expect("address").port()
    }

    #[test]
    fn assignments_are_remembered_across_loads() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut allocator = PortAllocator::load(temp.path()).expect("load");
        let port = allocator.allocate("api", None).expect("allocate");
        allocator.save().expect("save");

        let mut reloaded = PortAllocator::load(temp.path()).expect("reload");
        assert_eq!(reloaded.remembered("api"), Some(port));
        assert_eq!(reloaded.allocate("api", None).expect("reuse"), port);
    }

    #[test]
    fn fixed_ports_must_be_free() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut allocator = PortAllocator::load(temp.path()).expect("load");
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let busy = listener.local_addr().expect("address").port();

        let error = allocator
            .allocate("db", Some(busy))
            .expect_err("busy port is rejected");
        assert!(error.to_string().contains("already in use"), "{error}");

        let port = free_port();
        assert_eq!(allocator.allocate("db", Some(port)).expect("fixed"), port);
    }

    #[test]
    fn automatic_ports_are_unique_within_one_run() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut allocator = PortAllocator::load(temp.path()).expect("load");
        let first = allocator.allocate("a", None).expect("first");
        let second = allocator.allocate("b", None).expect("second");
        assert_ne!(first, second);
        assert!(is_free(first));
        assert!(is_free(second));
    }

    #[test]
    fn a_remembered_but_busy_port_is_replaced() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut allocator = PortAllocator::load(temp.path()).expect("load");
        let port = allocator.allocate("api", None).expect("allocate");
        allocator.save().expect("save");

        let mut reloaded = PortAllocator::load(temp.path()).expect("reload");
        let _listener = TcpListener::bind(("127.0.0.1", port)).expect("occupy remembered port");
        let replacement = reloaded.allocate("api", None).expect("replace");
        assert_ne!(replacement, port);
    }
}
