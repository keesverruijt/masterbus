//! Full end-to-end check of the engine: list devices, lazily discover every
//! menu, and print every group/field (with writability) plus the monitoring
//! values.
//!
//! Usage: `enumerate`
//!
//! Transport (USB / SocketCAN), master role, and the schema cache directory
//! come from the per-host config file ([`masterbus::FileConfig`]); the file
//! is created on first run.

use masterbus::{Config, MasterBus, Menu};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let bus = MasterBus::auto(Config::default()).unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    });

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
