//! Live end-to-end check of the engine over SocketCAN.
//!
//! Usage: `enumerate <can-interface> [cache-dir]` (Linux/SocketCAN only)
//! Lists alive devices, lazily discovers each (all menus), prints identity and
//! every group/field (with writability) and the monitoring values.

#[cfg(target_os = "linux")]
fn main() {
    use masterbus::{Config, MasterBus, Menu};

    let mut args = std::env::args().skip(1);
    let iface = args.next().unwrap_or_else(|| {
        eprintln!("usage: enumerate <can-interface> [cache-dir]");
        std::process::exit(1);
    });
    let cache = args.next().map(std::path::PathBuf::from);

    let config = Config { cache_path: cache, ..Default::default() };
    let bus = match MasterBus::socketcan(&iface, config) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(2);
        }
    };

    let devices = bus.devices_all();
    println!("{} device(s)", devices.len());
    for dev in devices {
        let name = dev.name().unwrap_or_default();
        let article = dev.article_number().unwrap_or_default();
        let serial = dev.serial_number().unwrap_or_default();
        let rev = dev.revision_code().unwrap_or_default();
        let fw = dev.firmware_version().unwrap_or_default();
        println!(
            "\n=== {} (id {}) art={} ser={} rev={} fw={} status={:?} ===",
            name, dev.id(), article, serial, rev, fw, dev.status()
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

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the `enumerate` example requires Linux/SocketCAN");
}
