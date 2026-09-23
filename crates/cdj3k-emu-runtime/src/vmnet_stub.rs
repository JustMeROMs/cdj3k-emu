//! Non-macOS placeholder for Apple's socket_vmnet integration.
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct SocketVmnet { socket_path: PathBuf }
impl SocketVmnet {
    pub fn start(_iface: &str) -> io::Result<Self> { Err(io::Error::new(io::ErrorKind::Unsupported, "socket_vmnet is macOS-only")) }
    pub fn start_bridged(_iface: &str) -> io::Result<Self> { Err(io::Error::new(io::ErrorKind::Unsupported, "socket_vmnet is macOS-only")) }
    pub fn start_host() -> io::Result<Self> { Err(io::Error::new(io::ErrorKind::Unsupported, "socket_vmnet is macOS-only")) }
    pub fn socket_path(&self) -> &Path { &self.socket_path }
}
pub(crate) fn other_clients_alive(_dir: &Path, _own: &Path) -> bool { false }
pub(crate) fn is_valid_iface(_iface: &str) -> bool { false }
