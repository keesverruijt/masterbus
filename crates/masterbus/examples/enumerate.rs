//! Full end-to-end check of the engine: list devices, lazily discover every
//! menu, and print every group/field (with writability) plus the monitoring
//! values.
//!
//! Usage (Linux):     `enumerate <can-iface>`     SocketCAN
//!                    `enumerate usb [serial]`    USB link
//! Usage (others):    `enumerate [serial]`        USB link (the only transport)
//!
//! Env: `CACHE_DIR=<path>`              on-disk schema cache (per device, by serial).
//!      `HEARTBEAT_MASTER=<24-bit hex>` drive the bus as master.

use masterbus::{Config, MasterBus, Menu};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config {
        heartbeat_master: std::env::var("HEARTBEAT_MASTER")
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()),
        cache_path: std::env::var_os("CACHE_DIR").map(Into::into),
        ..Config::default()
    };
    let bus = connect(&args, config);

    let devices = bus.devices_all();
    println!("{} device(s)", devices.len());
    for dev in devices {
        let name = dev.name().unwrap_or_default();
        let article = dev.article_number().unwrap_or_default();
        let serial = dev.serial_number().unwrap_or_default();
        let rev = dev.revision_code().unwrap_or_default();
        let fw = dev.firmware_version().unwrap_or_default();
        println!(
            "\n=== {} (id {:06X}) art={} ser={} rev={} fw={} status={:?} ===",
            name,
            dev.id(),
            article,
            serial,
            rev,
            fw,
            dev.status()
        );
        let groups = match dev.groups() {
            Ok(g) => g,
            Err(e) => {
                println!("  (schema error: {e})");
                continue;
            }
        };
        for g in groups {
            let menu = g.menu().unwrap_or(Menu::Other(0));
            println!("  [{:?}] {}", menu, g.name().unwrap_or_default());
            for f in g.fields().unwrap_or_default() {
                let fname = f.name().unwrap_or_default();
                let unit = f.unit().unwrap_or_default();
                let wr = if f.is_writable().unwrap_or(false) { "rw" } else { "ro" };
                // Read values only for the monitoring menu to limit bus load.
                let val = if menu == Menu::Monitoring {
                    match f.value() {
                        Ok(v) => format!("{:?}", v),
                        Err(_) => "-".into(),
                    }
                } else {
                    String::new()
                };
                println!("      {:>3} {:<24} {:<4} {} {}", f.index(), fname, unit, wr, val);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn connect(args: &[String], config: Config) -> MasterBus {
    let r = match args.first().map(String::as_str) {
        Some("usb") => MasterBus::usb(args.get(1).map(String::as_str), config),
        Some(iface) => MasterBus::socketcan(iface, config),
        None => {
            eprintln!("usage: enumerate <can-iface>      # SocketCAN");
            eprintln!("       enumerate usb [serial]     # USB link");
            eprintln!("       (env: CACHE_DIR, HEARTBEAT_MASTER)");
            std::process::exit(1);
        }
    };
    r.unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    })
}

#[cfg(not(target_os = "linux"))]
fn connect(args: &[String], config: Config) -> MasterBus {
    MasterBus::usb(args.first().map(String::as_str), config).unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    })
}
