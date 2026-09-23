//! Non-macOS placeholder for the macOS TAP bridge integration.
use std::io;

#[derive(Debug)]
pub struct TapBridge {
    pub bridge_iface: String,
    pub qemu_tap: String,
    pub qemu_tap_fd: i32,
}

impl TapBridge {
    pub fn setup(_host_tap: &str, _instance_id: u32) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "TAP bridge is not implemented on this host yet"))
    }
}

pub fn cleanup_stale(_instance_id: u32) {}
