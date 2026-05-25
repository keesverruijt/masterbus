# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- Cross-platform **USB-link transport** (`usb` feature, via `hidapi`):
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

### Fixed
- Address **direction bit** (`0x080000`): devices announce/respond with it set but
  must be addressed with it clear. Requests are now sent to the bit-clear address
  and device ids are canonicalised to it, so non-battery devices (which ignore the
  bit-set address) are reachable — previously only the lenient lithium batteries
  answered. Confirmed against MasterAdjust's own USB traffic.
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

[Unreleased]: https://github.com/keesverruijt/masterbus/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/keesverruijt/masterbus/releases/tag/v0.1.0
