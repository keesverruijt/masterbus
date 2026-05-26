//! Connect to the bus and list the devices that are heard.
//!
//! Usage (Linux):     `devices <can-iface>`     SocketCAN
//!                    `devices usb [serial]`    USB link
//! Usage (others):    `devices [serial]`        USB link (the only transport)
//!
//! Env: `HEARTBEAT_MASTER=<24-bit hex>` makes us drive the bus as master so
//! devices announce themselves — needed when no hardware master (e.g. an
//! EasyView) is present, otherwise some devices stay quiet.

use masterbus::{Config, MasterBus};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config {
        heartbeat_master: std::env::var("HEARTBEAT_MASTER")
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()),
        ..Config::default()
    };
    let bus = connect(&args, config);

    let snapshot = bus.devices().len();
    let settled = bus.devices_all();
    println!("devices() (snapshot):    {snapshot}");
    println!("devices_all() (settled): {}", settled.len());
    for d in &settled {
        println!("  {:06X}", d.id());
    }
}

#[cfg(target_os = "linux")]
fn connect(args: &[String], config: Config) -> MasterBus {
    let r = match args.first().map(String::as_str) {
        Some("usb") => MasterBus::usb(args.get(1).map(String::as_str), config),
        Some(iface) => MasterBus::socketcan(iface, config),
        None => {
            eprintln!("usage: devices <can-iface>      # SocketCAN");
            eprintln!("       devices usb [serial]     # USB link");
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
