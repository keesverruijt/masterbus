# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **`masterbus-signalk` publishes a curated mapping, not a built-in table.**
  `map_field` decided both *what* a field meant and *where* it went, by
  matching the device-class prefix and the field's name. A 28-device survey
  (#12) showed that cannot work: field names are installer-editable, two
  charger articles both advertise as `CHG` with unrelated contents under
  identically-named groups, and even factory names differ between models, so
  the supported `BAT` class was silently dropping voltage, current and
  temperature on one battery family. Publishing is now driven by
  `mapping.json` beside `config.ini`, keyed on device **serial number** and
  **field id** — what the firmware fixes rather than what a human typed.
  Presence is the toggle; there are no menu/group flags and so no precedence
  rules. Unit conversion is **derived** from the field's unit and the target
  leaf's unit rather than stored, so a mapping never carries a scale factor
  someone could get wrong; a pair that cannot be reconciled is reported at
  startup and skipped instead of published as a wrong number. `invert` is the
  one transform no unit can express. On the first run with no file, a mapping
  is seeded from the old per-class heuristics (now
  `masterbus_tools::seed`, with no authority over anything) and written out,
  so an existing install keeps working and has something to edit. The old
  `mapping.ini` is not migrated; its group toggles have no equivalent.
- **Per-installation Signal K paths.** Closes #3: every path in the mapping is
  an override, including onto a different category. The device's `name` and
  `manufacturer` metadata follow it, because the Signal K node is now derived
  from the paths a device actually publishes to rather than from a per-class
  table. A leaf this build knows no unit for is still published, with a
  warning that it carries no unit metadata.
- **One configuration directory per host.** `/etc/default/masterbus-signalk/`
  is gone; everything lives beside `config.ini` in `/etc/default/masterbus/`
  (and the OS-native per-user equivalent elsewhere). `config.ini` gained a
  `listen` key for the Signal K sidecar's address, so the systemd unit no
  longer needs an `EnvironmentFile=`, a second `ConfigurationDirectory=` or a
  second `ReadWritePaths=`. A command-line address still overrides the file.
  `FileConfig::mapping_path()` resolves the mapping file to that same
  directory, so the sidecar and the coming TUI editor cannot disagree about
  where it is. **Upgrading:** move `mapping.ini` into
  `/etc/default/masterbus/`, copy any `LISTEN=` into `config.ini`'s `listen`
  key, and delete the old directory. (#12)

### Added
- **A bundled per-model suggestion database.** Keyed on **article number** and
  field id, so it reaches the models the class-and-name heuristics cannot: the
  two charger articles in #6 both advertise as `CHG` with unrelated field sets,
  and one has had its output fields renamed by the installer. Suggestions are
  now tiered — an exact entry for this article and firmware, then one for the
  article, then the name guess — and the TUI says which tier a proposal came
  from, because "known for this model" and "guessed from a name" deserve
  different scrutiny. Every entry records its provenance. Ships with the two
  charger articles from #6; add more in
  `crates/masterbus-tools/src/suggestions/catalog.json`. (#12)
- **`masterbus-dump` includes the host's mapping.** Whole, and as a `signalk`
  path beside each mapped field, so a reported dump shows the bus and the
  decisions made about it together — which is what lets a user's curation
  become a bundled suggestion for the next person. Absent when there is no
  mapping file. (#12)
- **`masterbus-tui --mapping` edits the Signal K mapping.** The device list
  shows how many of each device's fields publish, the Monitoring tab gains a
  Signal K column, `+` and `-` map and unmap the selected field, `a` copies a
  device's mapping onto every other device with the same article, and `w`
  writes the file. The `+` prompt pre-fills from the existing mapping or the
  built-in heuristics and shows the conversion the path implies, which is the
  only place a human can check it, since the file stores no scale factor. A
  path whose units cannot be reconciled is refused rather than saved; an
  unfamiliar leaf is accepted with a note that it publishes without unit
  metadata. Copying skips fields the target device does not have, so a cluster
  master's extra fields are not forced onto a plain member, and it keeps an
  instance the user already chose. Entries for devices that are off the bus are
  preserved. A path too long for the row wraps onto a continuation line rather
  than being cut off, since the path is the whole point of the mode. (#12)
- **A library target for `masterbus-tools`.** The crate was binaries only,
  which left `masterbus-signalk` and `masterbus-tui` unable to share code. Two
  modules to start: `signalk` (the SI unit each Signal K path leaf carries) and
  `units` (the device-unit → Signal K conversion, *derived* from the pair of
  units rather than stored per field, so a curated mapping never has to record
  a scale factor a human could get wrong). Groundwork for #12.
- **`masterbus-dump`** — a fourth command-line tool that walks the whole bus and
  writes one JSON document: per device its identity and status, per group its id
  and menu, per field its channel-aware id, name, unit, range, enum options and
  writability, plus the live monitoring values. `--menus` picks which menus to
  enumerate, `--values none|monitoring|all` how much to read, `--device <hex>`
  restricts to one device, `--probe` adds the flat field-index probe. Intended
  for issue reports: it gives someone without the hardware everything needed to
  write a Signal K mapping. Field ids print as `0x000`..`0x1FF`, the same
  encoding `masterbus-tui` shows and `masterbus-set-field` accepts.
- **`CONTRIBUTING.md`** — a getting-started guide for non-developers: installing
  the Rust toolchain per OS, building, using an AI assistant to add a device
  class to the Signal K sidecar, and what "unsupported device" means (the core
  discovers every device; only `masterbus-signalk`'s per-class mapping is
  model-specific).
- **Events resolve their target device by name.** A device's event definitions
  (`Event N source / target / command / data`) reference their target device by
  a bare index that is a position in the address-sorted bus device list. Wire
  visualization code `0x09` (`DeviceList`) is now mapped (it previously fell
  through to `Float`, so targets showed as a raw number), and `masterbus-tui`
  renders it as the target's name (`→ Solar`) via the live device list. The
  action (`data`) already shows its label (`Off`/`On`/`Copy`/`Copy invert`/
  `Toggle`). See PROTOCOL §9a.
- **Events resolve their command to the target's output name.** The `Event N
  command` field (wire viz `0x0A`, now mapped) selects which of the target
  device's eventable outputs to drive. Per-field metadata op `0x0D` (eventable
  flag) is now fetched into `FieldInfo.eventable`, `Device::eventable_outputs()`
  returns a device's ordered output names, and `masterbus-tui` renders the
  command as that name (e.g. `Close relay`) when the target device's config has
  been discovered (else `output N`).

### Fixed
- **`CONTRIBUTING.md` was a guide to the wrong thing.** It opened by
  explaining that you build from source "typically to add a device class",
  installed Rust, built the workspace, and only afterwards revealed that
  fixing your own devices needs no code at all — while the one section about
  the library told you to capture a trace and let someone else fix it. It is
  now four sections for people changing the project: toolchain, build, a map
  of the codebase, and **Working on the library**, which says where a decode
  bug, a stalled enumeration or a transport problem actually lives, treats
  `docs/PROTOCOL.md` as part of the deliverable, and treats a frame trace as
  something you read rather than only attach to an issue. Anyone who wants
  their own bus in Signal K is sent to `ENDUSER.md` in the second paragraph.
  `ENDUSER.md` gained the detail that moved out, and the README now routes
  non-developers there rather than to the contributor guide.
- **`ENDUSER.md` said the sidecar publishes everything it finds, and told
  people to open an issue and wait.** Both were true before the curated
  mapping landed and neither is now: `masterbus-signalk` publishes exactly what
  `mapping.json` names, and the owner can fill that in themselves with
  `masterbus-tui --mapping` in a couple of minutes. The guide now leads with
  that step and says plainly that skipping it means no Signal K data,
  explains why it cannot be automatic, and keeps the dump-and-report flow as
  the way to help the *next* owner of a model rather than the way to help
  yourself. The `masterbus` crate README's platform table also still listed
  three tools.
- **`cargo doc` failed with warnings denied.** Two intra-doc links had rotted
  (`DeviceList` in the protocol module, `MasterBus::device_ids` in the TUI),
  which nothing else in the build notices. Fixed, and `make precommit` and CI
  now run `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings` so they cannot
  rot again.
- **The mapping editor records the instance a path actually uses.** Curated
  paths are typed by hand and often do not contain the instance proposed for
  the device: an `INT Nav Chg` gets mapped onto
  `electrical.chargers.nav-battery` because that is what it charges. `instance`
  is what "apply to this article" substitutes, so leaving it stale made that
  copy substitute nothing and hand two devices the same Signal K node. The
  instance is now taken from the committed path, and a copy that would
  substitute nothing is skipped rather than written. Found in real use. (#12)
- **The seed no longer maps two fields of one device onto the same Signal K
  path.** Found by deploying onto a live 14-device bus: a battery reports the
  same six measurements once for its cluster and once for itself, so the seed
  produced twelve entries writing six paths, coalescing to whichever arrived
  last. An alternator does the same for battery voltage across its Battery and
  Shunt groups, which the old code documented as harmless but which is visible
  and fixable now that mappings are per field. The lowest field id wins; the
  rest are left out for a human to add deliberately. (#12)
- **Device property strings are trimmed.** At least one shipping charger
  reports its article number with a trailing space (`"44010250 "`), which
  silently defeats every lookup keyed on it — the bundled string catalog, and
  any per-model mapping table. Article, serial, name and revision are now
  trimmed at the point they are fetched. Schema caches keyed on an untrimmed
  serial are re-fetched once.
- **`masterbus-dump` was missing from the release artifacts.** The CI staging
  loop copied only `masterbus-tui` and `masterbus-set-field`, so the one tool
  that `CONTRIBUTING.md` asks people to run when reporting an unmapped device
  could only be obtained by building from source — precisely the toolchain
  setup the binary releases exist to avoid. It is a plain cross-platform binary
  on the same `MasterBus::auto` entry point as `masterbus-set-field`, so it now
  ships in every artifact and tarball. The tool listings in `ENDUSER.md`,
  `CONTRIBUTING.md` and the `masterbus-tools` README were likewise still
  written for three binaries; all now cover four, the crate README documents
  `masterbus-dump`'s options, and `ENDUSER.md` gained a section on reporting a
  device the Signal K sidecar does not map yet. (#10)
- **systemd unit failed to start on a fresh install (exit 226/NAMESPACE).**
  `ReadWritePaths=` listed `/etc/default/masterbus-signalk` and
  `/etc/default/masterbus`, and systemd refuses to set up the sandbox when a
  listed path is missing — which it is until something creates it. The unit now
  declares both via `ConfigurationDirectory=` so systemd creates them before
  the first start, and `masterbus-signalk` creates the parent directory of
  `MAPPING` itself for non-systemd runs. (#5)
- **`VisualizationType::DeviceList` value decoded from the wrong byte.** It read
  `byte[0]` of the 4-byte value, but the selection is an `f32` index (like
  Radio/DropDown), so any index ≥ 1 was misread as `0`. Now decoded as
  `round(f32)`.
- **Discovery slowdown from the eventable (`0x0D`) query.** A device answers
  `0x0D` only for its eventable fields, so querying it blocking on every field
  paid a full metadata timeout on every silent (non-eventable) field — badly
  slowing monitoring-heavy devices. It is now a **best-effort** query (a short
  poll after the blocking ops, since the reply arrives with the batch) issued
  **only on Monitoring fields** (the only tab that carries eventable outputs).
  No behaviour change to the resolved output names; just no more per-field
  timeouts.

## [0.3.4] - 2026-07-26

### Added
- **Offline string-table catalog (discovery accelerator).** Devices serve their
  string table four characters per CAN round trip (opcode `0x30`), which
  dominates first-time discovery — an MLI Ultra reports ~400 strings. The static
  half of each table is baked into vendor firmware and identical across units of
  a given `(article, firmware)`, so it is now recovered offline and bundled
  (`crates/masterbus/src/strings/catalog.json`, generated and validated
  id-for-id against a live 14-device bus by `tools/gen_runtime.py` in the RE
  tree). On discovery the table is confirmed against the live device by a
  three-id spot-check before use; a stale, wrong-language, or wrong-revision
  table fails the check and degrades to the existing live fetch — never to a
  wrong string. Ships tables for the EasyView 52, Digital Input Switch, and MLI
  Ultra; unbundled models are unaffected. See the new `masterbus::strings`
  module and PROTOCOL §4.4.
- **`masterbus-signalk`: MAC `deviceMode`.** The MAC "Device state" enum
  (Standby/Charging/…) is published as `electrical.chargers.<id>.deviceMode`,
  alongside the existing charge-stage `chargingMode`.
- Unit tests for `map_field`, pinning the published MAC paths and the
  name/unit matching rules.

### Fixed
- **`masterbus-signalk`: MAC monitoring values were silently dropped.** The MAC
  schema reports an empty unit on several monitoring fields, so the strict
  (name, unit) match never fired and `voltage`, `current`, `input.current` and
  `voltageSense` were missing from the deltas even though `masterbus-tui`
  showed them. Those fields now also accept a missing unit; the temperature
  fields keep the strict match, since `Device` / `Battery` are told apart by
  their unit alone. Reported by @EdKok (#2).

## [0.3.3] - 2026-07-20

### Added
- **`masterbus-signalk`: two more device classes.** MAC DC-DC chargers map to
  `electrical.chargers.<id>` and APR alternator regulators to
  `electrical.alternators.<id>` (input/output/battery voltages & currents,
  temperatures, field current, shaft/engine RPM as Hz, and `chargingMode` from
  the charge-state enum).
- **`masterbus-signalk`: static device metadata.** Every published device now
  emits `name` and `manufacturer` (name + article/model) once per client
  connection.
- **`masterbus-signalk`: unit metadata.** Each published path emits a `meta`
  delta declaring its SI units, so Signal K can unit-convert the non-standard
  nested leaves (`battery.temperature`, `field.current`, `input.voltage`, …).
  Both metadata kinds are replayed to late-joining clients on connect.
- `Error::CommitFieldOccupied { field, cmd_field, cmd_field_name }` — relay-
  style boolean writes (CombiMaster inverter / charger) now hard-error when
  the adjacent commit slot at `field+1` is occupied by a *named* user-facing
  field, instead of silently failing to actuate. The value write is not
  emitted in that case, so no half-state on the bus.

### Fixed
- Relay-commit heuristic recognises the CombiMaster's hidden register at
  `field+1` even when the device reports it in schema discovery. On a
  CombiMaster, discovering the menu containing field 19/21 (inverter / charger
  toggles) populates the schema with an *unnamed* `FieldInfo` (empty name,
  zero min/max/step, no options) at field 20/22 — the commit register itself.
  Previously the heuristic treated that as a "real, in-the-way" field and
  skipped the commit token, so boolean writes were silently ignored by the
  device. Empty-name entries are now treated as hidden.

### Changed
- `masterbus::write` logging promoted from `debug` to `info` for the per-call
  entry / read-back pair (`→ write 0x… field 0x… = …`, `← read-back …`), with
  an additional info line noting whether the commit token was emitted or
  refused. Per-write activity is rare enough that info-level is appropriate.

### Changed
- `masterbus::discovery`: the per-menu "discovered N groups" log demoted from
  `info` to `debug`, and the per-attempt "giving up after N attempts" demoted
  from `warn` to `debug`. Neither is actionable during normal startup.

## [0.3.2] - 2026-05-28

### Added
- **Logging via the `log` facade.** The library emits `info`/`warn`/`error`/
  `debug`/`trace` on these targets: `masterbus` (default, connect /
  alive / offline), `masterbus::discovery` (per-menu enumeration, retries),
  `masterbus::cache` (schema-cache load/save), `masterbus::settings`
  (config-file creation, cache_dir fallback), `masterbus::write`
  (per-write debug), `masterbus::frame` (candump-style Tx/Rx dump on
  `trace`). No runtime cost when no backend is installed.
- The bundled binaries pull in `env_logger`. `masterbus-signalk` defaults
  to `info`; `masterbus-set-field` defaults to `warn`; `masterbus-tui`
  routes its logs to an in-app pane (toggle with `~`) via `tui-logger`,
  defaulting to `info`. Set `MASTERBUS_TUI_LOG=<path>` to redirect to a
  file instead.
- `masterbus::frame` trace lines carry a trailing semantic tag —
  `read-btm1`, `write-btm1`, `read-btm3`, `write-btm3`, `push-btm1`,
  `ack`, `poll`, `broadcast`, `schema-req`, `meta-btm3-req`, etc. —
  so reads, writes, pushes, and discovery requests are visually
  distinct without decoding the class byte by hand.
- TUI: press `?` on a list / eventable field to see all option labels
  with their underlying integer indices, current value highlighted.
- `MasterBus::auto(Config)` — one-call constructor that reads (and on
  first run creates) a per-host config file, then picks the right
  transport. The file lives at `/etc/default/masterbus/config.ini` on
  Linux (when writable) and otherwise at the OS-native per-user path
  (`$XDG_CONFIG_HOME/masterbus/...` on Linux, `~/Library/Application
  Support/masterbus/...` on macOS, `%APPDATA%\masterbus\...` on
  Windows). It carries four keys (`heartbeat_master`, `device_type`,
  `device_name`, `cache_dir`) and is auto-populated from the hardware
  on first run (USB link preferred; otherwise the lone CAN interface).
  The path and detected values are logged via `log::debug!` on creation.
- New public types: `masterbus::FileConfig`, `masterbus::DeviceType`.

### Removed
- `MASTERBUS_LOG` env var (point-at-a-file frame log). Replaced by
  `RUST_LOG=masterbus::frame=trace` — same data, candump-compatible
  format (`Tx 05000001 [0]` / `Rx 04188EA2 [8] 14 93 …`), routed through
  whatever `log` backend the consuming binary installs.

### Changed
- **All four binaries** (`masterbus-tui`, `masterbus-signalk`,
  `masterbus-set-field`, plus the examples) now use `MasterBus::auto`.
  The `<transport>` / `<can-iface>` positional argument, the
  `[cache-dir]` positional, the `CACHE_DIR` env var, and the
  `--heartbeat-master <hex>` flag are gone — edit the config file to
  change any of them. `masterbus-set-field` now takes only
  `<device_id> <field_id> <value>`; `masterbus-signalk` only
  `[listen-addr]`; `masterbus-tui` takes no arguments.
- `FileConfig` gains `cache_dir: Option<PathBuf>`. On creation the key
  defaults to `/var/lib/masterbus` for system installs and to the
  OS-native per-user cache directory otherwise (XDG on Linux,
  `~/Library/Caches/masterbus` on macOS, `%LOCALAPPDATA%\masterbus\cache`
  on Windows). At runtime, a non-writable configured path silently
  falls back to the per-user cache, so root-owned daemons and user-run
  tools share schemas when possible. Comment the key out to disable
  on-disk caching entirely.
- Per-user config path is now OS-native: `$XDG_CONFIG_HOME/masterbus/`
  on Linux (default `~/.config/masterbus/`), `~/Library/Application
  Support/masterbus/` on macOS, `%APPDATA%\masterbus\` on Windows.
  (Previous beta builds used `~/.local/masterbus/` everywhere — if you
  have such a file, copy it to the new location or let the tools
  re-create one.)
- The bundled systemd unit no longer sets `CAN_IFACE` or passes the
  cache path to `ExecStart` — both come from
  `/etc/default/masterbus/config.ini`. `ReadWritePaths` extended to
  include that directory.
- TUI right-pane field rows reordered to `tag · name · rw|ro · value · unit`
  with a 21-char value column; list values render as `Label(index)` so
  the raw wire value is visible alongside the human label.

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
