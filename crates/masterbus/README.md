# masterbus

Idiomatic Rust client for the Mastervolt **MasterBus** CAN-bus protocol — the
network used by Mastervolt marine power equipment (inverter/chargers, batteries,
alternator regulators, solar/charge controllers, displays).

It discovers the devices on the bus, reads and writes their fields, and streams
live values, over Linux SocketCAN (a USB-link transport is in progress). Two API
surfaces share one engine: a blocking navigator (`MasterBus` → `Device` →
`Group` → `Field`) for scripts and one-shot tools, and a non-blocking
channel/subscription API for TUIs and daemons.

```rust,no_run
# #[cfg(target_os = "linux")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use masterbus::{Config, MasterBus, Menu};

let bus = MasterBus::socketcan("can0", Config::default())?;
for device in bus.devices() {
    println!("{} (id {})", device.name()?, device.id());
    for group in device.tab(Menu::Monitoring)? {
        for field in group.fields()? {
            println!("  {} = {:?}", field.name()?, field.value()?);
        }
    }
}
# Ok(()) }
# #[cfg(not(target_os = "linux"))]
# fn main() {}
```

See the [repository](https://github.com/keesverruijt/masterbus) for the full
workspace (C ABI, TUI) and `docs/PROTOCOL.md` for the wire protocol.

## License

Apache-2.0.
