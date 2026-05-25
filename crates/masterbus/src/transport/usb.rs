//! Mastervolt USB-link transport (cross-platform, HID).
//!
//! The "MasterBus USB Link" (USB VID `0x1A64`) is a class-compliant HID device
//! that bridges the CAN bus. It needs no vendor driver, so this transport runs
//! on macOS, Windows and Linux (handy on machines without a CAN interface).
//!
//! ## Wire framing (reverse-engineered)
//!
//! Both directions use fixed **64-byte reports, report id 0**:
//!
//! ```text
//! byte 0      : N = number of valid 14-byte records that follow
//! bytes 1..   : N records, 14 bytes each:
//!                 [0]   hdr   = can_class << 3  (top 3 bits duplicate addr[23:21])
//!                 [1]   addr hi   (bits 23:16)
//!                 [2]   addr mid  (bits 15:8)
//!                 [3]   addr lo   (bits 7:0)
//!                 [4]   dlc       (0..=8)
//!                 [5..13] data, zero-padded to 8 bytes
//!                 [13]  trailing (unused, 0)
//! tail        : device-appended trailer (ignored)
//! ```
//!
//! `can_id = (can_class << 24) | addr24`. A single 64-byte report can batch up to
//! four CAN frames (host→device we only ever emit one per report).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hidapi::{HidApi, HidDevice};

use super::{Transport, TransportRx, TransportTx};
use crate::error::{Error, Result};

/// Mastervolt USB vendor id.
const VID: u16 = 0x1A64;
const REPORT_LEN: usize = 64;
const REC_LEN: usize = 14;
const MAX_DATA: usize = 8;
/// Cap each blocking HID read so the writer can't be starved of the shared lock.
const READ_CAP: Duration = Duration::from_millis(50);

/// Mastervolt USB-link transport (HID).
pub struct UsbTransport {
    dev: Arc<Mutex<HidDevice>>,
}

impl UsbTransport {
    /// Open the first MasterBus USB Link, or the one with the given serial number.
    pub fn open(serial: Option<&str>) -> Result<Self> {
        let api = HidApi::new().map_err(|e| Error::Connection(format!("hidapi: {e}")))?;
        let info = api
            .device_list()
            .filter(|d| d.vendor_id() == VID)
            .find(|d| match serial {
                Some(want) => d.serial_number() == Some(want),
                None => true,
            })
            .ok_or_else(|| {
                Error::Connection(match serial {
                    Some(s) => format!("no MasterBus USB Link with serial {s}"),
                    None => "no MasterBus USB Link found".into(),
                })
            })?;
        let dev = info
            .open_device(&api)
            .map_err(|e| Error::Connection(format!("open USB link: {e}")))?;
        Ok(UsbTransport { dev: Arc::new(Mutex::new(dev)) })
    }
}

impl Transport for UsbTransport {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (
            Box::new(Rx { dev: self.dev.clone(), queue: VecDeque::new() }),
            Box::new(Tx { dev: self.dev }),
        )
    }
}

/// Split a 64-byte input report into its CAN frames.
fn parse_report(report: &[u8], out: &mut VecDeque<(u32, Vec<u8>)>) {
    let count = report.first().copied().unwrap_or(0) as usize;
    for i in 0..count {
        let base = 1 + i * REC_LEN;
        let Some(rec) = report.get(base..base + REC_LEN) else { break };
        let class = u32::from(rec[0] >> 3);
        let addr = (u32::from(rec[1]) << 16) | (u32::from(rec[2]) << 8) | u32::from(rec[3]);
        let dlc = (rec[4] as usize).min(MAX_DATA);
        let can_id = (class << 24) | addr;
        out.push_back((can_id, rec[5..5 + dlc].to_vec()));
    }
}

/// Build a 64-byte output report carrying a single CAN frame, prefixed with the
/// report-id byte (0) that `hid_write` requires.
fn build_report(can_id: u32, data: &[u8]) -> [u8; REPORT_LEN + 1] {
    let mut buf = [0u8; REPORT_LEN + 1];
    buf[0] = 0x00; // report id
    let p = &mut buf[1..]; // 64-byte payload
    p[0] = 1; // one record
    p[1] = ((can_id >> 21) & 0xFF) as u8; // hdr = class<<3 | addr[23:21]
    p[2] = ((can_id >> 16) & 0xFF) as u8;
    p[3] = ((can_id >> 8) & 0xFF) as u8;
    p[4] = (can_id & 0xFF) as u8;
    let dlc = data.len().min(MAX_DATA);
    p[5] = dlc as u8;
    p[6..6 + dlc].copy_from_slice(&data[..dlc]);
    buf
}

struct Rx {
    dev: Arc<Mutex<HidDevice>>,
    queue: VecDeque<(u32, Vec<u8>)>,
}

impl TransportRx for Rx {
    fn recv(&mut self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>> {
        if let Some(frame) = self.queue.pop_front() {
            return Ok(Some(frame));
        }
        let mut report = [0u8; REPORT_LEN];
        let ms = timeout.min(READ_CAP).as_millis() as i32;
        let n = {
            let dev = self.dev.lock().unwrap();
            dev.read_timeout(&mut report, ms)
                .map_err(|e| Error::Connection(format!("USB read: {e}")))?
        };
        if n == 0 {
            return Ok(None);
        }
        parse_report(&report[..n], &mut self.queue);
        Ok(self.queue.pop_front())
    }
}

struct Tx {
    dev: Arc<Mutex<HidDevice>>,
}

impl TransportTx for Tx {
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
        let buf = build_report(can_id, data);
        let dev = self.dev.lock().unwrap();
        dev.write(&buf).map_err(|e| Error::Connection(format!("USB write: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_records() {
        // count=2, then a class-0x18 request (dlc 2) and a class-0x08 value (dlc 6).
        let mut r = [0u8; REPORT_LEN];
        r[0] = 2;
        // record 0: c3 6a 96 cf 02 8b 00 ...
        r[1..1 + REC_LEN].copy_from_slice(&[0xc3, 0x6a, 0x96, 0xcf, 0x02, 0x8b, 0x00, 0, 0, 0, 0, 0, 0, 0]);
        // record 1: 43 6a 96 cf 06 8b 00 bb 49 de 41 ...
        r[15..15 + REC_LEN]
            .copy_from_slice(&[0x43, 0x6a, 0x96, 0xcf, 0x06, 0x8b, 0x00, 0xbb, 0x49, 0xde, 0x41, 0, 0, 0]);
        let mut q = VecDeque::new();
        parse_report(&r, &mut q);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0], (0x18_6a_96_cf, vec![0x8b, 0x00]));
        assert_eq!(q[1], (0x08_6a_96_cf, vec![0x8b, 0x00, 0xbb, 0x49, 0xde, 0x41]));
    }

    #[test]
    fn build_round_trips_class_and_addr() {
        let buf = build_report(0x18_6a_96_cf, &[0x8c, 0x00, 0x00, 0x00, 0x10, 0x41]);
        let p = &buf[1..];
        assert_eq!(buf[0], 0x00); // report id
        assert_eq!(p[0], 1); // one record
        assert_eq!(p[1], 0xc3); // class 0x18 << 3 | addr[23:21]=0b011
        assert_eq!(&p[2..5], &[0x6a, 0x96, 0xcf]);
        assert_eq!(p[5], 6); // dlc
        assert_eq!(&p[6..12], &[0x8c, 0x00, 0x00, 0x00, 0x10, 0x41]);
    }
}
