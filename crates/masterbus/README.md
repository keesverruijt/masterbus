# masterbus

Rust library for the Mastervolt **MasterBus** CAN-bus protocol — the network
used by Mastervolt marine power equipment (inverter/chargers, batteries,
alternator regulators, solar/charge controllers, displays).

It discovers the devices on the bus, reads and writes their fields, and streams
live values. Two API surfaces share one engine: a blocking navigator
(`MasterBus` → `Device` → `Group` → `Field`) for scripts and one-shot tools,
and a non-blocking channel/subscription API for TUIs and daemons.

## Transports & OS availability

Two transports ship out of the box:

| Transport | Build target | Constructor | Notes |
|---|---|---|---|
| **SocketCAN** | Linux only | `MasterBus::socketcan("can0", config)` | Native Linux CAN. Works with any SocketCAN-capable adapter (CANable + candleLight, PiCAN HAT, MCP2515-based DIY boards, the kernel's `vcan`, etc.). Bring up at **250 kbit/s**. |
| **Mastervolt USB link** | Linux, macOS, Windows | `MasterBus::usb(serial, config)` | The Mastervolt USB Interface (vendor article 77030200) — a class-compliant HID device, no vendor driver needed. `serial` is `None` for "first one found" or `Some("…")` to pin a specific adapter. |

| Crate / OS | Linux | macOS | Windows |
|---|---|---|---|
| `masterbus` library | ✅ SocketCAN **and** USB link | ✅ USB link | ✅ USB link |
| `masterbus-tools` binaries (`-tui`, `-signalk`, `-set-field`) | ✅ | ✅ tui + set-field (signalk needs Linux/SocketCAN) | ✅ tui + set-field (same caveat) |
| `masterbus-ffi` C ABI | ✅ | ✅ | ✅ |

SocketCAN is compiled in only when `target_os = "linux"`; the USB transport is
built unconditionally so the same code base works on a Raspberry Pi wired to
the bus or a developer laptop using the USB interface.

## Example

```rust,no_run
# #[cfg(target_os = "linux")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use masterbus::{Config, MasterBus, Menu};

// SocketCAN on Linux, or `MasterBus::usb(None, Config::default())` for
// the Mastervolt USB Interface on any of Linux / macOS / Windows.
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
workspace (C ABI, TUI, Signal K sidecar, one-shot CLI writer) and
`docs/PROTOCOL.md` for the wire protocol.

## License

Apache-2.0.
