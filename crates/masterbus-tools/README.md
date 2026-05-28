# masterbus-tools

Three command-line tools for Mastervolt **MasterBus**, built on the
[`masterbus`](https://crates.io/crates/masterbus) library. Install all
of them in one go:

    cargo install masterbus-tools

Each binary works over **Linux SocketCAN** or — on Linux, macOS, and
Windows — over the **Mastervolt USB link** (a class-compliant HID
device, no vendor driver needed).

## Configuration

All three tools share a small INI file describing the transport and the
optional "act as bus master" role:

| OS | Config path | Default cache dir |
|----|------|------|
| Linux (system) | `/etc/default/masterbus/config.ini` (if writable) | `/var/lib/masterbus` |
| Linux (user) | `$XDG_CONFIG_HOME/masterbus/config.ini` (default `~/.config/...`) | `$XDG_CACHE_HOME/masterbus` (default `~/.cache/...`) |
| macOS | `~/Library/Application Support/masterbus/config.ini` | `~/Library/Caches/masterbus` |
| Windows | `%APPDATA%\masterbus\config.ini` | `%LOCALAPPDATA%\masterbus\cache` |

The file is **read on every start** and **auto-created on first run**
with sensible defaults — a Mastervolt USB link if plugged in, otherwise
the lone CAN interface. The chosen path and detected values are logged
to stderr on creation.

```ini
heartbeat_master = 000001               # 24-bit hex; comment out to stay passive
device_type      = can                  # "usb" or "can"
device_name      = can0                 # CAN iface, or USB-link serial (blank = first)
cache_dir        = /var/lib/masterbus   # schema cache; comment out to disable
```

Multiple CAN interfaces with no USB link is an error — edit the file
and pick one. To switch transports, change the master role, or relocate
the cache, edit the file (or delete it to re-auto-detect). If the
configured `cache_dir` isn't writable by the running user (e.g.
`/var/lib/masterbus` for an unprivileged shell), the engine silently
falls back to the OS-native per-user cache directory.

## `masterbus-tui`

Terminal UI for browsing devices, viewing live values, and editing
writable fields.

    masterbus-tui

Devices are listed on the left with liveness; the selected device's
groups and fields are on the right. `Tab` / `Shift-Tab` switch between
the Summary / Monitoring / Configuration / Service / Settings tabs (each
discovered on demand). `Enter` edits a writable field — booleans toggle,
numbers / lists / text open a centred edit modal. `l` opens the
access-level (login) modal — higher levels unlock more fields. `q`
quits.

## `masterbus-signalk`

[Signal K](https://signalk.org) sidecar: subscribes to the monitoring
values of every device and serves Signal K deltas as newline-delimited
JSON over TCP (default `0.0.0.0:3009`), with values converted to SI
units. The instance id is the device's name, lowercased and stripped of
its leading class word (e.g. `BAT Main Batt 4` → `main-batt-4`).

    masterbus-signalk [listen-addr]
    # e.g.: masterbus-signalk                # default port (0.0.0.0:3009)
    #       masterbus-signalk 0.0.0.0:4000   # bind elsewhere

Sample delta:

```json
{"updates":[{"$source":"masterbus","timestamp":"2026-05-25T18:00:00.000Z","values":[{"path":"electrical.batteries.main-batt-4.voltage","value":26.6}]}]}
```

### Filtering with `mapping.ini`

If the `MAPPING` environment variable points at a file, output is gated
per device/group. As devices are discovered the file is auto-populated
and rewritten; **edit the `true`/`false` flags while the service is
stopped**, then restart.

Keys are `<instance>.<menu>[.<group>] = true|false`. A group-level line
overrides the menu-level line. New entries default **off**, except a
battery's `cluster` group, which defaults **on** — so out of the box
you get the battery cluster and nothing else:

```ini
# main-batt — groups: battery, cluster
main-batt.monitoring = false
main-batt.monitoring.cluster = true

# combimaster — groups: ac-in, ac-out, dc-in-out, general
combimaster.monitoring = false
```

Without `MAPPING`, every mapped field is published.

### Run as a systemd service

A hardened unit is included at
[`etc/masterbus-signalk.service`](etc/masterbus-signalk.service):

```sh
sudo cp $(which masterbus-signalk) /usr/local/bin/             # already there if installed via cargo install
sudo cp etc/masterbus-signalk.service /etc/systemd/system/
# First run as root creates /etc/default/masterbus/config.ini with
# auto-detected transport + master settings; review/edit it as needed.
sudo systemctl enable --now masterbus-signalk
```

Transport, master role, and the schema-cache directory are configured
in `/etc/default/masterbus/config.ini` (auto-created on first run; see
the **Configuration** section above) and the systemd unit's `LISTEN`
env var. The service keeps a persistent schema cache in
`/var/lib/masterbus` and restarts on failure.

## `masterbus-set-field`

One-shot CLI to write a single field — handy from shell scripts and
cron:

    masterbus-set-field <device_id> <field_id> <value>

- `<device_id>`: hex 24-bit address from the TUI's title bar, e.g. `188EA2`.
- `<field_id>`: three-digit hex from the TUI field list, e.g. `0x013`
  (Btm1) or `0x10E` (Btm3 — bit 8 selects the channel).
- `<value>`: parsed per the field's type — boolean (`true`/`false`/`on`/
  `off`/`1`/`0`), number, list index *or* exact option label, or free
  text (max 16 printable-ASCII bytes for editable strings, the wire
  limit).

Examples:

    masterbus-set-field 188EA2 0x013 on               # CombiMaster bool
    masterbus-set-field 3A3B4B 0x104 "Nav Chg"        # Magic Nav Chg rename
    masterbus-set-field 53A493 0x160 "Schakelaar"     # EasyView Switch 1

## License

Apache-2.0.
