# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-05-28

First tagged release on the 0.3 line (0.3.0 was bumped in source but
never released).

### Added
- `Device::cached_access_level() -> Option<AccessLevel>` — non-blocking
  read of the engine-cached access level, safe to call from a render
  loop. The existing `Device::access_level()` still does a wire
  round-trip.

### Changed
- **Login is now password-based**; the library is value-opaque.
  `Device::login(level, code: f32)` takes whatever f32 the caller
  hands in; the crate packs it onto the wire as-is. `AccessLevel::code()`
  removed (the vendor-defined codes are no longer baked into the
  library or its tests). PROTOCOL.md §4.5 reduced to (level byte,
  label) — codes are noted as vendor-defined and out of scope. The
  TUI login modal is now two-stage: pick a level, then enter a
  password (chars rendered as `•`); the buffer is silently parsed
  as `f32` and submitted. If the device reports the same level it
  was at, the status line says *"that seems to be an incorrect
  password"*.
- TUI device list: a logged-in device's row appends ` (<Level>)` to
  its name (e.g. `Nav Chg (Installer)`) — read from the cached
  access level.
- TUI field rows: unit column widened from 4 → 5 chars (fits `°C`,
  `kWh`, `Hz` and similar with a trailing space).
- Reader: `State::touch` is now gated on the CAN class being
  device-originated (broadcast / response / push / ack). Our own
  loopback (`Dn 05/07/18/19/1A/1B/1C ...`) and any other master
  polling the bus no longer create spurious entries in the device
  list. Devices that don't broadcast `0x04` (e.g. an EasyView while
  another master polls it) still appear via their reply / push
  frames.

## [0.3.0] - 2026-05-28

### Added
- **Two-channel field model**: `Channel::Btm1` / `Btm3` exposed through a
  `FieldId = u16` (bit 8 = channel, bits 0..8 = wire index). The crate now
  speaks both the legacy Btm1 metadata path and the newer Btm3 path
  end-to-end. Visible in the TUI as the three-digit hex tag (`0x004`,
  `0x10E`, …) next to each editable row.
- **Active Btm3 value reads**: `btm3_read_raw` + an active read in
  `poll_value`. On a quiet bus (no other master polling), Btm3 fields
  used to never deliver a value because the device only pushes in
  response to a read; the crate now issues that read itself.
- **Editable string round-trip**: `Value::Text { sid, text }` carries the
  string id of the editable slot; `Field::set(Value::Text { … })` writes
  via the class-`0x07` chunk protocol (PROTOCOL.md §4.4). A wire-level
  16-byte printable-ASCII cap (`MAX_EDITABLE_TEXT_BYTES`) is validated
  at the API boundary and enforced in the TUI editor.
- **Per-device access-level login** (`Field::set_access_level` /
  `Device::login` / `Device::logout`, opcode `0x08 0x19` on class
  `0x07`). The schema cache is keyed by access level so writability
  flips on level change don't poison the cache.
- **`masterbus-tools` crate** that ships three binaries — `masterbus-tui`,
  `masterbus-signalk`, `masterbus-set-field` — via one
  `cargo install masterbus-tools`. The library crate `masterbus` stays
  lean; library consumers (`cargo add masterbus`) don't pull in
  `ratatui` / `serde_json`.
- **`masterbus-set-field`** binary: one-shot CLI write for shell
  scripts (`masterbus-set-field <transport> <device_id> <field_id> <value>`,
  values parsed against the field's discovered visualization).
- **Optional CAN frame log** via `MASTERBUS_LOG=<path>` (Tx + Rx, one
  frame per line, vmware.log-compatible decoded form). Cheap when
  disabled.
- **TUI tab redesign**: Summary / Monitoring / Configuration / Service /
  Settings. Every tab subscribes so values stream in; the Settings tab
  enumerates the Btm3 flat field list and presents per-field metadata
  not otherwise reachable from the per-menu Btm1 schema.
- **TUI edit modal**: editing moved out of the bottom status line into
  a centred 60×9 popup (mirroring the login modal). Text edits pre-fill
  with the current string and show a live char counter against the
  16-byte cap.
- **Touch-creates-device** in `State::touch`: any inbound frame from a
  device registers it. Silent-but-polled devices (e.g. an EasyView
  responding to another master's queries without emitting class-`0x04`
  broadcasts) now appear in the device list.
- `Disc::probe_existence`: pipelined existence sweep in 64-frame chunks
  paced by `min_send_interval` (default 1 ms). Bridges multi-index
  gaps in the Btm3 field-id space (the EasyView has a 66-index gap
  between header fields and the Switch block) that the previous
  miss-streak loop skipped, finishing a 256-index sweep in ~1 s per
  channel.

### Changed
- **Renamed "shadow" → "Btm1/Btm3 metadata"** throughout the wire
  abstraction. The shadow concept (`addr | 0x800000`) is now an
  internal address detail of the Btm1 metadata path; no public symbol
  named "shadow" remains.
- `State::has_field` / `viz_of` / `Field::info` / `Engine::ensure_field`
  consult both `schema.groups` (Btm1 menu walk) and `all_fields` (Btm3
  flat probe). Previously every Btm3 field returned `FieldNotAvailable`
  to `Field::set`.
- `Config::min_send_interval` default `3 ms → 1 ms`, safe at 250 kbit/s
  with the chunked probe.

### Fixed
- TUI Configuration tab now subscribes to its fields (was: lazy-on-
  click) so values are present when the user clicks "edit".
- Reader caches Btm3 value pushes via the unified `state.field_info`,
  not the old menu-groups-only lookup. Btm3 values stopped getting
  silently dropped.
- TUI device list now masks the Btm1-shadow flag (`0x800000`) on
  inbound traffic so `0xBA3B4B` and `0x3A3B4B` don't create two
  entries.

### Removed
- The standalone `masterbus-tui`, `masterbus-signalk`, and
  `masterbus-set-field` workspace crates — their binaries now live
  under `masterbus-tools`. The published `masterbus-tui` crate (and
  any siblings) will be yanked from crates.io.

## [0.2.0] - 2026-05-25

### Added
- `masterbus-signalk`: a Signal K sidecar that streams MasterBus monitoring
  values as Signal K deltas (newline-delimited JSON) over **TCP** (default
  `0.0.0.0:3009`), with SI unit conversion. Ships with a hardened systemd unit.
- `masterbus-signalk`: per-device publish control via a `mapping.ini` (set
  `MAPPING`, or use the config dir `/etc/default/masterbus-signalk/`). Entries
  are `<instance>.<menu>[.<group>] = true|false`; new devices are auto-added
  (menu off, the battery `cluster` group on) and the file is rewritten on
  discovery. Edit flags while the service is stopped.
- `Device::identity()` / `Device::tab_info()` and `DeviceIdentity` for cheap,
  per-menu access without full discovery.
- List/enum values now carry their **option labels**: the engine fills a list
  field's `Value` with the schema's option strings, and `Value::index()` /
  `Value::label()` return the numeric selection and its meaning. `Field::info()`
  is public so callers can also get the full `FieldInfo` (options, bounds, …).
- **Bus-master heartbeat**: with `Config::heartbeat_master` set (signalk:
  `HEARTBEAT_MASTER=<hex>`), the scheduler periodically emits a class-`0x05`
  heartbeat so devices announce (class `0x04`) and stay responsive — needed to
  enumerate the bus when no hardware master (e.g. an EasyView) is present.
- Cross-platform **USB-link transport** (via `hidapi`, always built):
  `MasterBus::usb()` talks to the class-compliant "MasterBus USB Link" HID device
  (VID `0x1A64`) directly — no vendor driver/DLL — so the crate runs on
  macOS/Windows and on Linux hosts without a CAN interface. Includes an `usb`
  example (`enumerate`/`dump`/`read`/`write`).

### Changed
- Rust **edition 2024** across the workspace (MSRV 1.85); updated dependencies
  (`thiserror` 1→2, `ratatui` 0.29→0.30, `cbindgen` 0.27→0.29) and committed
  `Cargo.lock` for reproducible binary builds.
- The schema cache is now keyed **per device (serial)** instead of by
  article+firmware. Devices that share an article can differ (e.g. one battery
  in a cluster exposes an extra "Cluster" group), so the old key could hide
  those differences.
- TUI: tab-lazy discovery (render Monitoring immediately, load other tabs on
  demand); non-blocking boot with live device-name backfill.
- TUI now runs on **macOS/Windows** over the USB link with **no argument**
  (the only transport there); on Linux pass `<can-iface>` or `usb [serial]`.
- CI now also builds **Windows** (`x86_64-pc-windows-msvc`, statically linked)
  and **macOS** (`aarch64`/`x86_64-apple-darwin`) alongside the Linux targets.
- USB transport is built unconditionally (no `usb` feature). On Linux it uses
  hidapi's pure-Rust hidraw backend (`linux-native-basic-udev`) so cross-builds
  need no `libudev`/C toolchain.

### Fixed
- Boolean/list writes now send the field's full **4-byte value** (a `CheckBox`
  is a float `1.0`/`0.0`), matching MasterAdjust; the old 1-byte boolean write
  was ignored by e.g. the CombiMaster's inverter/charger. `CheckBox` reads now
  decode any non-zero value as true (an "on" can arrive as float `1.0`, whose
  byte 0 is `0`).
- Relay-style boolean controls now actually switch: after the value write the
  scheduler emits a fixed **commit token** (`14 9f 3c 02`) to the adjacent hidden
  command register at `field+1` — captured from MasterAdjust toggling the
  CombiMaster inverter/charger (constant across both, on/off). Only sent when
  `field+1` is not a real schema field, so it never clobbers a neighbour.
- **List/dropdown** values are the selected index as a 4-byte **float** (e.g.
  option 1 = `1.0`), not a low-byte integer — so both reads and writes of
  drop-downs (e.g. the Solar "Override" enable) now match the device.
- Battery "Cluster" group no longer hidden: dropping the cross-device schema
  dedup means each battery (including the cluster master) is discovered on its
  own.
- Discovery is dramatically faster: lazy per-menu discovery and not re-fetching
  unused numeric bounds; routing class-0x10 "no value" replies.
- `connect()` waits up to 15 s (was 3 s) for the first broadcast, so a noisy bus
  no longer fails to start spuriously.

## [0.1.0] - 2026-05-25

Initial release.

### Added
- `masterbus` core library: the MasterBus CAN protocol, lazy per-menu discovery
  with an optional on-disk schema cache, a passive value cache with rate-based
  subscriptions, and a blocking navigator API (`MasterBus`/`Device`/`Group`/
  `Field`) plus a non-blocking channel/event API. Linux SocketCAN transport.
- `masterbus-ffi`: a single-threaded C ABI (`cdylib`/`staticlib`) with a
  cbindgen-generated header and C demos (`mb_enumerate`, `mb_get_value`,
  `mb_set_value`).
- `masterbus-tui`: a ratatui terminal UI to browse devices/values and edit
  writable settings.
- CI building release binaries for `x86_64`, `armhf` and `aarch64`, attached to
  tagged releases.

[Unreleased]: https://github.com/keesverruijt/masterbus/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/keesverruijt/masterbus/compare/v0.2.0...v0.3.1
[0.3.0]: https://github.com/keesverruijt/masterbus/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/keesverruijt/masterbus/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/keesverruijt/masterbus/releases/tag/v0.1.0
