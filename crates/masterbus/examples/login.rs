//! Per-device access-level login (PROTOCOL.md §4.5).
//!
//! Reads the device's current level, optionally logs in at a new level, and
//! reads the level back. Verifies the writability of one field before and
//! after so the effect of the change is visible.
//!
//! Usage (Linux):
//!     login <can-iface> <device-hex>                   # read current level
//!     login <can-iface> <device-hex> <level> [field]   # log in, dump writability
//!     login <can-iface> <device-hex> logout [field]
//!
//! `<level>` is one of: enduser, installer, distributor, mvservice
//! (case-insensitive). `[field]` is an optional field index to query
//! `is_writable` on; defaults to `0x17` (the CombiMaster AC IN limit field
//! used in the reference captures).
//!
//! USB: same as `devices`: `login usb [serial] <device-hex> ...`.

use std::time::Duration;

use masterbus::{AccessLevel, Config, Device, FieldId, MasterBus};

fn parse_level(s: &str) -> Option<AccessLevel> {
    match s.to_ascii_lowercase().as_str() {
        "enduser" | "0" | "logout" => Some(AccessLevel::EndUser),
        "installer" | "1" => Some(AccessLevel::Installer),
        "distributor" | "2" => Some(AccessLevel::Distributor),
        _ => None,
    }
}

fn dump_writable(device: &Device, field: FieldId) {
    match device.field(field).info() {
        Ok(info) => println!(
            "  field 0x{:04X} ({}): writable = {}",
            field, info.name, info.writeable
        ),
        Err(e) => println!("  field 0x{:04X}: {}", field, e),
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config {
        heartbeat_master: std::env::var("HEARTBEAT_MASTER")
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()),
        ..Config::default()
    };
    let bus = connect(&mut args, config);

    if args.is_empty() {
        eprintln!("missing <device-hex> argument");
        std::process::exit(1);
    }
    let dev_id = u32::from_str_radix(args[0].trim_start_matches("0x"), 16)
        .expect("device id must be 24-bit hex (e.g. 188EA2)");
    let device = bus.device(dev_id);

    println!("device 0x{:06X}", dev_id);
    match device.access_level() {
        Ok(l) => println!("current level: {:?}", l),
        Err(e) => {
            eprintln!("access_level read failed: {e}");
            std::process::exit(2);
        }
    }

    let field: FieldId = args
        .get(2)
        .and_then(|s| FieldId::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x17);

    if let Some(level_arg) = args.get(1) {
        let level = parse_level(level_arg).unwrap_or_else(|| {
            eprintln!("unknown level: {level_arg}");
            std::process::exit(1);
        });
        println!("\npre-{:?} writability:", level);
        dump_writable(&device, field);

        match device.login(level) {
            Ok(reported) => println!("login({:?}) -> device reports {:?}", level, reported),
            Err(e) => {
                eprintln!("login failed: {e}");
                std::process::exit(2);
            }
        }

        // Give the device a moment to update its metadata attributes before we
        // re-query writability (the post-login bursts in the reference capture
        // showed responses arriving in tens of milliseconds).
        std::thread::sleep(Duration::from_millis(100));
        println!("\npost-{:?} writability:", level);
        dump_writable(&device, field);
    }
}

#[cfg(target_os = "linux")]
fn connect(args: &mut Vec<String>, config: Config) -> MasterBus {
    let r = match args.first().map(String::as_str) {
        Some("usb") => {
            let serial_or_dev = args.get(1).cloned();
            // Distinguish `login usb <device>` from `login usb <serial> <device>`.
            let (serial, drop_n) = match &serial_or_dev {
                Some(s) if !looks_like_device_hex(s) => (Some(s.clone()), 2),
                _ => (None, 1),
            };
            args.drain(..drop_n);
            MasterBus::usb(serial.as_deref(), config)
        }
        Some(iface) => {
            // Clone the iface before mutating `args` so we don't hold an
            // immutable borrow into the same Vec across `remove`.
            let iface = iface.to_string();
            args.remove(0);
            MasterBus::socketcan(&iface, config)
        }
        None => {
            eprintln!("usage: login <can-iface> <device-hex> [level] [field-hex]");
            eprintln!("       login usb [serial] <device-hex> [level] [field-hex]");
            std::process::exit(1);
        }
    };
    r.unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    })
}

#[cfg(not(target_os = "linux"))]
fn connect(args: &mut Vec<String>, config: Config) -> MasterBus {
    // First arg may be a serial or the device hex. Heuristic: a 24-bit hex
    // (6 chars, all hex) is a device id; anything else is a serial.
    let (serial, drop_n) = match args.first() {
        Some(s) if !looks_like_device_hex(s) => (Some(s.clone()), 1),
        _ => (None, 0),
    };
    args.drain(..drop_n);
    MasterBus::usb(serial.as_deref(), config).unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    })
}

fn looks_like_device_hex(s: &str) -> bool {
    let t = s.trim_start_matches("0x");
    t.len() == 6 && t.chars().all(|c| c.is_ascii_hexdigit())
}
