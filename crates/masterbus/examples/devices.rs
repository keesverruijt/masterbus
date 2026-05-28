//! Connect to the bus and list the devices that are heard.
//!
//! Usage: `devices`
//!
//! Transport (USB / SocketCAN) and master role come from the per-host config
//! file ([`masterbus::FileConfig`]); the file is created on first run.

use masterbus::{Config, MasterBus};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let bus = MasterBus::auto(Config::default()).unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    });

    let snapshot = bus.devices().len();
    let settled = bus.devices_all();
    println!("devices() (snapshot):    {snapshot}");
    println!("devices_all() (settled): {}", settled.len());
    for d in &settled {
        println!("  {:06X}", d.id());
    }
}
