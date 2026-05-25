//! Mastervolt USB-link transport (cross-platform, HID).
//!
//! The USB link is a HID device (the vendor uses `libmastervolt_usb_interface.dll`
//! / `TCommMasterBusUsbInterface`). This lets the crate run on macOS/Windows and
//! on Linux machines without a CAN interface.
//!
//! **Not yet functional**: the HID report ↔ CAN-frame framing still needs to be
//! reverse-engineered (see the plan's deferred items). The type exists so the
//! `Transport` abstraction is complete; `open` currently returns an error.

use std::time::Duration;

use super::{Transport, TransportRx, TransportTx};
use crate::error::{Error, Result};

/// Mastervolt USB-link transport (HID).
pub struct UsbTransport {
    // hidapi device handle will live here once the framing is implemented.
}

impl UsbTransport {
    /// Open the first matching USB link, or one with the given serial number.
    pub fn open(_serial: Option<&str>) -> Result<Self> {
        Err(Error::Connection(
            "USB-link transport not yet implemented (HID framing pending RE)".into(),
        ))
    }
}

impl Transport for UsbTransport {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (Box::new(Rx {}), Box::new(Tx {}))
    }
}

struct Rx {}
impl TransportRx for Rx {
    fn recv(&mut self, _timeout: Duration) -> Result<Option<(u32, Vec<u8>)>> {
        Err(Error::Connection("USB-link transport not yet implemented".into()))
    }
}

struct Tx {}
impl TransportTx for Tx {
    fn send(&mut self, _can_id: u32, _data: &[u8]) -> Result<()> {
        Err(Error::Connection("USB-link transport not yet implemented".into()))
    }
}
