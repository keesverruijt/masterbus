//! Per-device access-level login (PROTOCOL.md §4.5).
//!
//! Reads the device's current level, optionally attempts a login (with a
//! caller-supplied password) or a logout, and reads the level back.
//! Verifies the writability of one field before and after so the effect of
//! a successful login is visible.
//!
//! Usage:
//!     login <device-hex>                             # read current level
//!     login <device-hex> <level> <password> [field]  # try login
//!     login <device-hex> logout [field]              # logout
//!
//! `<level>` is one of: installer, distributor, mvservice (case-insensitive).
//! `<password>` is parsed as a number (e.g. `123`, `-45.6`); whatever value
//! that produces is sent on the wire as-is — this crate is opaque to the
//! vendor-defined codes. If the device's reported level after the request
//! equals the *previous* level, the password was rejected silently.
//! `[field]` is an optional field index to query `is_writable` on; defaults
//! to `0x17`.
//!
//! Transport (USB / SocketCAN) and master role come from the per-host config
//! file ([`masterbus::FileConfig`]); the file is created on first run.

use std::time::Duration;

use masterbus::{AccessLevel, Config, Device, FieldId, MasterBus};

fn parse_level(s: &str) -> Option<AccessLevel> {
    match s.to_ascii_lowercase().as_str() {
        "installer" | "1" => Some(AccessLevel::Installer),
        "distributor" | "2" => Some(AccessLevel::Distributor),
        "mvservice" | "service" | "3" => Some(AccessLevel::MvService),
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: login <device-hex> [<level> <password> | logout] [field]");
        std::process::exit(1);
    }
    let bus = MasterBus::auto(Config::default()).unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    });
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

    if let Some(level_arg) = args.get(1) {
        let prev = device.access_level().ok();

        // `logout` (or `enduser` / `0`) takes no password and exits the
        // current level. Anything else needs a password argument.
        let logout = matches!(level_arg.to_ascii_lowercase().as_str(), "enduser" | "0" | "logout");
        let (level, code) = if logout {
            (AccessLevel::EndUser, None)
        } else {
            let level = parse_level(level_arg).unwrap_or_else(|| {
                eprintln!("unknown level: {level_arg}");
                std::process::exit(1);
            });
            let pw = args.get(2).cloned().unwrap_or_else(|| {
                eprintln!("missing <password> for level {level:?}");
                std::process::exit(1);
            });
            // Silent parse — whatever bytes the user types end up on the
            // wire; a malformed number becomes 0.0.
            (level, Some(pw.parse::<f32>().unwrap_or(0.0)))
        };
        let field: FieldId = args
            .get(if logout { 2 } else { 3 })
            .and_then(|s| FieldId::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0x17);

        println!("\npre-attempt writability:");
        dump_writable(&device, field);

        let reported = match code {
            None => device.logout(),
            Some(c) => device.login(level, c),
        };
        match reported {
            Ok(r) => {
                if Some(r) == prev {
                    println!("\nlevel unchanged ({r:?}) — that seems to be an incorrect password");
                } else {
                    println!("\nnow logged in as {r:?}");
                }
            }
            Err(e) => {
                eprintln!("login failed: {e}");
                std::process::exit(2);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        println!("\npost-attempt writability:");
        dump_writable(&device, field);
    }
}
