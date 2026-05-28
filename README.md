# masterbus

An idiomatic Rust client for the Mastervolt **MasterBus** CAN-bus protocol, with
a terminal UI, a Signal K sidecar, and a C-ABI wrapper. Talks to the bus over
**Linux SocketCAN**, or over the **Mastervolt USB link** (a class-compliant HID
device — no vendor driver) on **Linux, macOS, and Windows**.

A Cargo workspace:

| crate | what |
|-------|------|
| [`masterbus`](crates/masterbus) | core library (the protocol, discovery, caching, subscriptions, the `MasterBus`/`Device`/`Group`/`Field` navigator API and a non-blocking channel API) |
| [`masterbus-tools`](crates/masterbus-tools) | three command-line tools — `masterbus-tui` (terminal UI), `masterbus-signalk` (Signal K sidecar), `masterbus-set-field` (one-shot field writer). `cargo install masterbus-tools` installs all three |
| [`masterbus-ffi`](crates/masterbus-ffi) | C ABI `cdylib` (single-threaded), header generated with cbindgen, plus C demos (not published to crates.io) |

The wire protocol is documented in [`docs/PROTOCOL.md`](docs/PROTOCOL.md).

## Background

For years I've been struggling with the following. After some struggles with
Mastervolt Gel batteries that died way too quickly we did a major refit in 2019
at the boatyard with the electrical system being upgraded. All existing devices
were shipped back to Mastervolt and they signed off on the new setup with
4 x 5000 Wh Lithium batteries. Cost an arm and a leg, but it's been worth it.
Anyway, the customer success engineer that was helping me acknowledged that
programming their MasterBus/NMEA2000 interface was a bit difficult, so he
mentioned a new library that they were testing and was I interested in it? Sure!
They provided me with a "libmasterbus.so/dll" for Linux and Windows. It turned
out to be written in Rust, starting in 2017! Amazing. This library served me well
over the years, but I never received an update.

I tried contacting MasterVolt, but I learned that after their merger into what is
now Brunswick that everyone I knew no longer works there, and I can't get them to
reply to my queries about updates. Since the library was released under an
"Unlicense", e.g. fully open and free, and I really needed an Aarch64 version, I
decided last week that maybe I should just let Claude help me write a new version.
This repository is the fruit of that work.

## Design highlights

- **Fast init**: `connect()` returns as soon as the bus is usable and one device
  is heard — no multi-second enumeration wait, so one-shot tools stay snappy.
- **Lazy, per-menu discovery** with an optional on-disk schema cache (per device,
  keyed by serial) — only the menus you look at are discovered, and long-running
  programs discover each device once and load from cache thereafter.
- **Live values without polling storms**: a passive value cache fed by bus
  traffic, plus rate-based subscriptions that actively poll only what's needed.
- **Two API surfaces over one core**: a blocking navigator API for simple
  one-shots, and a non-blocking channel/event API for the TUI and daemons.

## Building

The core ships two transports: **Linux SocketCAN** (for hosts wired to the bus —
e.g. a Raspberry Pi) and the **Mastervolt USB link** (a class-compliant HID
device — no vendor driver — usable on Linux, macOS, and Windows). SocketCAN is
compiled in only on Linux; the USB transport is built unconditionally.

```sh
cargo build --workspace                 # build everything (host)
cargo test  --workspace                 # unit tests
cargo build --release --target aarch64-unknown-linux-gnu   # cross-compile for a Pi
```

Or install the three command-line tools (`masterbus-tui`,
`masterbus-signalk`, `masterbus-set-field`) directly:

```sh
cargo install masterbus-tools
```

### TUI

```sh
# Linux: SocketCAN, or the USB link
masterbus-tui can0 [cache-dir]               # e.g. masterbus-tui can0 ~/.cache/masterbus
masterbus-tui usb  [serial] [cache-dir]      # over the Mastervolt USB link

# macOS / Windows: USB link only (no argument needed)
masterbus-tui
```

Browse devices on the left; the selected device's tabs are on the right —
`Summary` / `Monitoring` / `Configuration` / `Service` / `Settings`, each
discovered on demand. `Tab` / `Shift-Tab` switch tabs, `Enter` edits a writable
field (booleans toggle, numbers / lists / text open a centred edit modal),
`l` opens the access-level (login) modal — higher levels unlock more fields,
`q` quits.

### Signal K sidecar

`masterbus-signalk` is a long-running service that publishes MasterBus monitoring
values as **Signal K deltas** (newline-delimited JSON) over TCP (default
`0.0.0.0:3009`), with SI-unit conversion. A `mapping.ini` controls which
devices/menus/groups are published; new devices are auto-added with sane defaults.
Ships with a hardened systemd unit.

```sh
masterbus-signalk <can-interface> [listen-addr] [cache-dir]
# e.g. masterbus-signalk can0 0.0.0.0:3009 /var/lib/masterbus
```

To act as the bus master (so devices keep responding when no hardware master is
present), set `HEARTBEAT_MASTER=<24-bit hex>` in the environment.

### C library and demos

`masterbus-ffi` builds a `libmasterbus_ffi` C library; `include/masterbus.h` is
generated by cbindgen. The C demos under
[`crates/masterbus-ffi/c`](crates/masterbus-ffi/c) (`mb_enumerate`,
`mb_get_value`, `mb_set_value`) build natively by default, with a `make remote`
target to cross-compile and deploy to a Pi. See that directory's `Makefile`.

## Status

Works over SocketCAN on Linux and over the Mastervolt USB link on Linux, macOS,
and Windows. The navigator API, FFI demos, TUI, and Signal K sidecar have all
been exercised against a live bus.

## TODO

- [ ] **Alarms tab / event streams** — `Menu::Alarm` and `Menu::History` are
  defined and discoverable, but the TUI tabs for them were removed during the
  channel-aware redesign and not yet restored. The crate also doesn't surface
  alarm broadcasts as events to API consumers.
- [ ] **`masterbus-set-field` Date / Time writes** — currently rejected; not a
  common need but the CLI advertises the omission.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

The original Mastervolt `libmasterbus` was released under the Unlicense (public
domain); this is an independent reimplementation.
