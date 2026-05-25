//! CAN transport abstraction.
//!
//! The protocol/discovery/cache/subscription engine is written against these
//! traits, so it runs unchanged over either a Linux SocketCAN interface or the
//! cross-platform Mastervolt USB link (a HID device).
//!
//! A frame is a raw 29-bit extended id plus 0–8 data bytes: `(u32, Vec<u8>)`.

use std::time::Duration;

use crate::error::Result;

/// A connected CAN transport, split into independent read/write halves so the
/// reader thread and the bus scheduler can use the wire concurrently.
pub trait Transport {
    /// Split into a receive half and a transmit half.
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>);
}

/// The receive half of a [`Transport`].
pub trait TransportRx: Send {
    /// Receive the next frame, blocking up to `timeout`. `Ok(None)` on timeout.
    fn recv(&mut self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>>;
}

/// The transmit half of a [`Transport`].
pub trait TransportTx: Send {
    /// Send a raw extended-CAN frame.
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()>;
}

#[cfg(target_os = "linux")]
pub mod socketcan;

pub mod usb;
