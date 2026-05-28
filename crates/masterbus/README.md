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

| OS | Config path | Default cache dir |
|----|------|------|
| Linux (system) | `/etc/default/masterbus/config.ini` (if writable) | `/var/lib/masterbus` |
| Linux (user) | `$XDG_CONFIG_HOME/masterbus/config.ini` (default `~/.config/...`) | `$XDG_CACHE_HOME/masterbus` (default `~/.cache/...`) |
| macOS | `~/Library/Application Support/masterbus/config.ini` | `~/Library/Caches/masterbus` |
| Windows | `%APPDATA%\masterbus\config.ini` | `%LOCALAPPDATA%\masterbus\cache` |

```ini
heartbeat_master = 000001               # 24-bit hex, or comment out to stay passive
device_type      = can                  # "usb" or "can"
device_name      = can0                 # CAN iface, or USB-link serial (blank = first)
cache_dir        = /var/lib/masterbus   # schema cache; comment out to disable
```

On creation, a Mastervolt USB link is preferred; otherwise the lone CAN
interface is selected. `cache_dir` defaults to the OS-native per-user cache
(see the table above) — or `/var/lib/masterbus` when the file is created in
`/etc/default/masterbus/` — and silently falls back to the per-user path at
runtime when the requested path isn't writable by the running user. The
chosen path and detected values are logged to stderr on creation.

### Logging

The crate emits diagnostics via the standard [`log`](https://crates.io/crates/log)
facade — install any compatible backend (`env_logger`, `tracing-log`, …) in
your binary. Useful targets:

| Target | Levels |
|---|---|
| `masterbus` (default) | `info` — device alive/offline, connect; `error` — fatal connect failures |
| `masterbus::discovery` | `info` — per-menu enumeration completion; `debug` — retries / timeouts; `warn` — retries exhausted |
| `masterbus::cache` | `debug` — saved files; `warn` — write/encode errors |
| `masterbus::settings` | `debug` — config-file creation, cache_dir fallback |
| `masterbus::write` | `debug` — every `do_write` call |
| `masterbus::frame` | `trace` — per-frame candump-style Tx/Rx (high volume) |

Quick start with `env_logger`:

```rust,no_run
env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
let bus = masterbus::MasterBus::auto(masterbus::Config::default())?;
# Ok::<_, Box<dyn std::error::Error>>(())
```

`RUST_LOG=masterbus::frame=trace` gives a candump-style line per frame:

```text
Tx  05000001   [0]
Rx  04188EA2   [8]  14 93 A4 03 D5 01 00 00
```

See the [repository](https://github.com/keesverruijt/masterbus) for the full
workspace (C ABI, TUI, Signal K sidecar, one-shot CLI writer) and
`docs/PROTOCOL.md` for the wire protocol.

## License

Apache-2.0.
