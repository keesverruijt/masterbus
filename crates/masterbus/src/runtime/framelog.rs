//! Optional CAN-frame log. When the `MASTERBUS_LOG` env var is set to a file
//! path, every Tx/Rx frame is appended in `vmware.log`-compatible decoded form
//! (one frame per line):
//!
//! ```text
//! Up  04 53A493  [8]  14 93 A4 03 D5 01 00 00  (2026-05-28T14:30:01.234567Z)
//! ```
//!
//! Cheap when disabled: the env var is read once, then a `None` is cached and
//! every call is a single atomic load.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn sink() -> Option<&'static Mutex<File>> {
    SINK.get_or_init(|| {
        let path = std::env::var("MASTERBUS_LOG").ok()?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

/// Log a frame. `dir` is `"Up"` (received) or `"Dn"` (transmitted).
pub fn log(dir: &str, can_class: u8, device_addr: u32, data: &[u8]) {
    let Some(sink) = sink() else { return };
    let mut buf = String::with_capacity(96);
    buf.push_str(dir);
    buf.push_str("  ");
    buf.push_str(&format!("{:02X} {:06X}  [{}]  ", can_class, device_addr & 0x00FF_FFFF, data.len()));
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            buf.push(' ');
        }
        buf.push_str(&format!("{:02X}", b));
    }
    buf.push_str("  (");
    buf.push_str(&iso_now());
    buf.push_str(")\n");
    if let Ok(mut f) = sink.lock() {
        let _ = f.write_all(buf.as_bytes());
    }
}

fn iso_now() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs();
    let us = now.subsec_micros();
    // crude UTC ISO-8601 (Z) — sufficient for line-grain ordering
    let (h, m, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let days = secs / 86400;
    // Days since 1970-01-01 → date. Simple non-leap-aware-ish formula good enough
    // for short-window diagnostics; the wall-clock seconds part is exact.
    let (y, mo, d) = ymd_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z", y, mo, d, h, m, s, us)
}

fn ymd_from_days(mut days: u64) -> (u32, u32, u32) {
    let mut y = 1970u32;
    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        y += 1;
    }
    let dim = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1u32;
    for &n in &dim {
        if days < n {
            break;
        }
        days -= n;
        mo += 1;
    }
    (y, mo, (days + 1) as u32)
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Public façade for non-runtime callers.
pub(crate) fn frame_log(dir: &str, can_class: u8, device_addr: u32, data: &[u8]) {
    log(dir, can_class, device_addr, data)
}
