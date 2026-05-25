//! Verify `devices_all()` returns the whole bus.
//!
//! Usage: `devices <can-interface>` (Linux/SocketCAN only)

#[cfg(target_os = "linux")]
fn main() {
    use masterbus::{Config, MasterBus};

    let iface = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: devices <can-interface>");
        std::process::exit(1);
    });
    let bus = match MasterBus::socketcan(&iface, Config::default()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(2);
        }
    };
    let quick = bus.devices().len();
    let all = bus.devices_all();
    let ids: Vec<u32> = all.iter().map(|d| d.id()).collect();
    println!("quick devices(): {quick}");
    println!("devices_all(): {} -> {:?}", ids.len(), ids);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the `devices` example requires Linux/SocketCAN");
}
