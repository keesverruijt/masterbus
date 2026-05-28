//! Watch the bus: print device presence events and live field updates.
//!
//! Demonstrates the two long-running APIs of this crate:
//!
//! * [`MasterBus::device_events`] — a `Receiver<DeviceEvent>` that fires
//!   `Alive` when a device first appears and `Offline` when it stops
//!   broadcasting within the liveness window.
//! * [`MasterBus::subscribe`] (and the equivalent [`Field::subscribe`])
//!   — a polled stream of [`ValueUpdate`]s for a chosen set of fields,
//!   keyed by `(device, field)` and labelled with the decoded [`Value`].
//!
//! Usage (Linux):    `watch <can-iface>`     SocketCAN
//!                   `watch usb [serial]`    USB link
//! Usage (others):   `watch [serial]`        USB link (the only transport)
//!
//! Env: `HEARTBEAT_MASTER=<24-bit hex>` makes us drive the bus as master
//! so devices announce themselves — needed when no hardware master is
//! present.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use masterbus::{Config, DeviceEvent, DeviceId, MasterBus, Menu, Subscription};

/// How often the engine re-polls each subscribed field. The cached value
/// is served straight back if it was refreshed within this window.
const POLL: Duration = Duration::from_millis(1_000);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config {
        heartbeat_master: std::env::var("HEARTBEAT_MASTER")
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()),
        ..Config::default()
    };
    let bus = connect(&args, config);

    // device_events: a single `Receiver<DeviceEvent>` for the whole bus. We
    // re-use the same receiver clone for the lifetime of the program.
    let events = bus.device_events();

    // One subscription per device, kept alive in this map; dropping the
    // `Subscription` value unsubscribes from the engine (see Drop impl).
    let subs: Arc<Mutex<HashMap<DeviceId, Subscription>>> = Arc::new(Mutex::new(HashMap::new()));

    println!("watching… ctrl-c to quit");
    while let Ok(ev) = events.recv() {
        match ev {
            DeviceEvent::Alive(id) => {
                println!("+ 0x{id:06X} alive");
                // Discover the device's Monitoring tab (cheap once cached) and
                // subscribe to every field in it; updates stream in via the
                // returned `Subscription`'s receiver.
                let device = bus.device(id);
                let Ok(groups) = device.tab(Menu::Monitoring) else { continue };
                let mut fields = Vec::new();
                for g in &groups {
                    if let Ok(fs) = g.fields() {
                        for f in fs {
                            fields.push(f.index());
                        }
                    }
                }
                if fields.is_empty() {
                    continue;
                }
                let sub = bus.subscribe(id, fields, POLL, /*change_only=*/ true);
                // Move the receiver into a worker thread that prints each
                // update. The `Subscription` itself stays parked in `subs`
                // so a future `Offline` can drop it (and trigger unsubscribe).
                let rx = sub.receiver().clone();
                subs.lock().unwrap().insert(id, sub);
                thread::spawn(move || {
                    while let Ok(u) = rx.recv() {
                        println!(
                            "  0x{:06X}  field 0x{:03X}  = {:?}",
                            u.device, u.field, u.value
                        );
                    }
                });
            }
            DeviceEvent::Offline(id) => {
                println!("- 0x{id:06X} offline");
                // Dropping the `Subscription` calls `engine.unsubscribe(id)`,
                // and the worker thread above exits cleanly when its
                // `Receiver` channel closes.
                subs.lock().unwrap().remove(&id);
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
            eprintln!("usage: watch <can-iface>        # SocketCAN");
            eprintln!("       watch usb [serial]       # USB link");
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
    let serial = args.first().map(String::as_str);
    MasterBus::usb(serial, config).unwrap_or_else(|e| {
        eprintln!("connect failed: {e}");
        std::process::exit(2)
    })
}
