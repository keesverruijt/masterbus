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
//!                 [0]   hdr  = (can_class << 3) | addr[23:21]
//!                 [1]   addr high byte, scrambled (see `decode_can_id`)
//!                 [2]   addr mid  (bits 15:8, plain)
//!                 [3]   addr lo   (bits 7:0, plain)
//!                 [4]   dlc       (0..=8)
//!                 [5..13] data, zero-padded to 8 bytes
//!                 [13]  trailing (unused, 0)
//! tail        : device-appended trailer (ignored)
//! ```
//!
//! `can_id = (can_class << 24) | addr24`, reconstructed so it matches the bus /
//! SocketCAN view. A single 64-byte report can batch up to four CAN frames
//! (host→device we only ever emit one per report).

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
        Ok(UsbTransport {
            dev: Arc::new(Mutex::new(dev)),
        })
    }
}

impl Transport for UsbTransport {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (
            Box::new(Rx {
                dev: self.dev.clone(),
                queue: VecDeque::new(),
            }),
            Box::new(Tx { dev: self.dev }),
        )
    }
}

/// Decode a 14-byte record's header into the real 29-bit CAN id.
///
/// The HID framing scrambles the **address high byte** across the header byte
/// and a second byte; the mid/low bytes are plain. (`b1` bit 3 is a direction
/// marker — set on device→host, clear on host→device — and is not part of the
/// address.) This reconstructs the same id the bus / SocketCAN sees.
fn decode_can_id(rec: &[u8]) -> u32 {
    let class = u32::from(rec[0] >> 3);
    let addr_hi = (u32::from(rec[0] & 0x07) << 5)
        | ((u32::from(rec[1] >> 5) & 0x07) << 2)
        | u32::from(rec[1] & 0x03);
    let addr = (addr_hi << 16) | (u32::from(rec[2]) << 8) | u32::from(rec[3]);
    (class << 24) | addr
}

/// Split a 64-byte input report into its CAN frames.
fn parse_report(report: &[u8], out: &mut VecDeque<(u32, Vec<u8>)>) {
    let count = report.first().copied().unwrap_or(0) as usize;
    for i in 0..count {
        let base = 1 + i * REC_LEN;
        let Some(rec) = report.get(base..base + REC_LEN) else {
            break;
        };
        let can_id = decode_can_id(rec);
        let dlc = (rec[4] as usize).min(MAX_DATA);
        out.push_back((can_id, rec[5..5 + dlc].to_vec()));
    }
}

/// Build a 64-byte output report carrying a single CAN frame, prefixed with the
/// report-id byte (0) that `hid_write` requires. Inverse of [`decode_can_id`]
/// (host→device, so the direction-marker bit is left clear).
fn build_report(can_id: u32, data: &[u8]) -> [u8; REPORT_LEN + 1] {
    let class = (can_id >> 24) & 0x1F;
    let addr_hi = (can_id >> 16) & 0xFF;
    let mut buf = [0u8; REPORT_LEN + 1];
    buf[0] = 0x00; // report id
    let p = &mut buf[1..]; // 64-byte payload
    p[0] = 1; // one record
    p[1] = ((class << 3) | (addr_hi >> 5)) as u8; // hdr = class<<3 | addr[23:21]
    p[2] = ((((addr_hi >> 2) & 0x07) << 5) | (addr_hi & 0x03)) as u8; // scrambled addr[20:16]
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
        dev.write(&buf)
            .map_err(|e| Error::Connection(format!("USB write: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_real_addresses() {
        // Real captured records → the same ids SocketCAN reports.
        // Battery 6E96CF: hdr c3, b1 6a.
        assert_eq!(
            decode_can_id(&[0xc3, 0x6a, 0x96, 0xcf, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            0x18_6E_96_CF
        );
        // CombiMaster 188EA2: hdr c0, b1 c8 (device→host marker bit set).
        assert_eq!(
            decode_can_id(&[0xc0, 0xc8, 0x8e, 0xa2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            0x18_18_8E_A2
        );
        // Class-04 broadcast of the CombiMaster: hdr 0x20.
        assert_eq!(
            decode_can_id(&[0x20, 0xc8, 0x8e, 0xa2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            0x04_18_8E_A2
        );
    }

    #[test]
    fn parse_two_records() {
        let mut r = [0u8; REPORT_LEN];
        r[0] = 2;
        // class-0x18 request (dlc 2) and class-0x08 value (dlc 6) from battery 6E96CF.
        r[1..1 + REC_LEN].copy_from_slice(&[
            0xc3, 0x6a, 0x96, 0xcf, 0x02, 0x8b, 0x00, 0, 0, 0, 0, 0, 0, 0,
        ]);
        r[15..15 + REC_LEN].copy_from_slice(&[
            0x43, 0x6a, 0x96, 0xcf, 0x06, 0x8b, 0x00, 0xbb, 0x49, 0xde, 0x41, 0, 0, 0,
        ]);
        let mut q = VecDeque::new();
        parse_report(&r, &mut q);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0], (0x18_6E_96_CF, vec![0x8b, 0x00]));
        assert_eq!(
            q[1],
            (0x08_6E_96_CF, vec![0x8b, 0x00, 0xbb, 0x49, 0xde, 0x41])
        );
    }

    #[test]
    fn build_matches_masteradjust_write() {
        // MasterAdjust's AC-IN-limit write to the CombiMaster was: 01 c0 c0 8e a2 06 17 00 00 00 a0 40
        let buf = build_report(0x18_18_8E_A2, &[0x17, 0x00, 0x00, 0x00, 0xa0, 0x40]);
        let p = &buf[1..];
        assert_eq!(buf[0], 0x00); // report id
        assert_eq!(&p[0..5], &[0x01, 0xc0, 0xc0, 0x8e, 0xa2]);
        assert_eq!(p[5], 6); // dlc
        assert_eq!(&p[6..12], &[0x17, 0x00, 0x00, 0x00, 0xa0, 0x40]);
    }

    #[test]
    fn build_then_decode_round_trips() {
        for id in [
            0x18_18_8E_A2u32,
            0x07_43_DF_24,
            0x18_6E_96_CF,
            0x04_30_47_2A,
        ] {
            let buf = build_report(id, &[1, 2, 3]);
            assert_eq!(decode_can_id(&buf[2..16]), id);
        }
    }
}
