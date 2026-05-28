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
| `masterbus-tools` binaries (`-tui`, `-signalk`, `-set-field`) | ✅ | ✅ (USB link only) | ✅ (USB link only) |
| `masterbus-ffi` C ABI | ✅ | ✅ | ✅ |

SocketCAN is compiled in only when `target_os = "linux"`; the USB transport is
built unconditionally so the same code base works on a Raspberry Pi wired to
the bus or a developer laptop using the USB interface.

## Example

```rust,no_run
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use masterbus::{Config, MasterBus, Menu};

// One-call connect using the per-host config file (auto-created on first run;
// see `masterbus::FileConfig`). Or call `MasterBus::socketcan(iface, …)` /
// `MasterBus::usb(serial, …)` directly to bypass the file.
let bus = MasterBus::auto(Config::default())?;
for device in bus.devices() {
    println!("{} (id {})", device.name()?, device.id());
    for group in device.tab(Menu::Monitoring)? {
        for field in group.fields()? {
            println!("  {} = {:?}", field.name()?, field.value()?);
        }
    }
}
# Ok(()) }
```

### Per-host config file

`MasterBus::auto` reads (and on first run, creates) a small INI:

| OS | Path |
|----|------|
| Linux (system) | `/etc/default/masterbus/config.ini` (if writable) |
| Linux (user) / macOS / Windows | `$HOME/.local/masterbus/config.ini` |

```ini
heartbeat_master = 000001               # 24-bit hex, or comment out to stay passive
device_type      = can                  # "usb" or "can"
device_name      = can0                 # CAN iface, or USB-link serial (blank = first)
cache_dir        = /var/lib/masterbus   # schema cache; comment out to disable
```

On creation, a Mastervolt USB link is preferred; otherwise the lone CAN
interface is selected. `cache_dir` defaults to `/var/lib/masterbus` for the
system file or `$HOME/.cache/masterbus` for the user file, and silently falls
back to `$HOME/.cache/masterbus` at runtime when the requested path isn't
writable by the running user. The path and detected values are logged to
stderr on creation.

See the [repository](https://github.com/keesverruijt/masterbus) for the full
workspace (C ABI, TUI, Signal K sidecar, one-shot CLI writer) and
`docs/PROTOCOL.md` for the wire protocol.

## License

Apache-2.0.
