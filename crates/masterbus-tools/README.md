# masterbus-tools

Four command-line tools for Mastervolt **MasterBus**, built on the
[`masterbus`](https://crates.io/crates/masterbus) library. Install all
of them in one go:

    cargo install masterbus-tools

Each binary works over **Linux SocketCAN** or — on Linux, macOS, and
Windows — over the **Mastervolt USB link** (a class-compliant HID
device, no vendor driver needed).

## Configuration

All four tools share a small INI file describing the transport and the
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

Terminal UI for browsing devices, viewing live values, editing writable
fields, and curating the Signal K mapping.

    masterbus-tui
    masterbus-tui --mapping   # also edit mapping.json; see the sidecar below

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
units. Which field lands where is a curated file, not a built-in table;
see below. When that file is seeded, the instance id proposed for a
device is its name lowercased and stripped of its leading class word
(e.g. `BAT Main Batt 4` → `main-batt-4`).

    masterbus-signalk [listen-addr]
    # e.g.: masterbus-signalk                # config.ini's `listen`, else 0.0.0.0:3009
    #       masterbus-signalk 0.0.0.0:4000   # bind elsewhere

Sample delta:

```json
{"updates":[{"$source":"masterbus","timestamp":"2026-05-25T18:00:00.000Z","values":[{"path":"electrical.batteries.main-batt-4.voltage","value":26.6}]}]}
```

### What gets published: `mapping.json`

The sidecar publishes exactly what the mapping file says, and nothing
else. It lives beside `config.ini`; `MAPPING` overrides the location.

Entries are keyed on the device's **serial number** and the **field id**,
because those are what the firmware fixes. Device, group and field
*names* are installer-editable — a charger's `Output 1` is routinely
renamed `Eng.batt` — and even factory names differ between models of the
same class, so nothing here matches on a name.

```json
{
  "version": 1,
  "devices": {
    "R516V1070": {
      "article": "26024000",
      "firmware": "2.65",
      "name": "MSU Inverter",
      "instance": "inverter",
      "fields": {
        "0x006": { "path": "electrical.inverters.inverter.dc.voltage" },
        "0x015": { "path": "electrical.chargers.inverter.enabled", "invert": true }
      }
    }
  }
}
```

Presence is the toggle: a field that is not listed is not published.
Field ids are the same three-digit hex the TUI shows next to every row.

There are no scale factors, on purpose. The conversion to SI follows
from the field's own unit and the unit the target path's leaf wants, so
`°C` into a `temperature` leaf becomes kelvin without being told. A pair
that cannot be reconciled is reported at startup and skipped rather than
published as a wrong number. `invert` is the one transform no unit can
express: a charger reporting `Standby` publishes to `enabled` negated.

The path is yours. Point a field at a non-standard leaf or a different
category and it is honoured; the device's `name` and `manufacturer`
metadata follow it there. A leaf this build knows no unit for is still
published, with a warning that it will carry no unit metadata.

**First run.** With no mapping file, the service seeds one from built-in
per-class name heuristics and writes it out, so an install keeps working
and has something to edit. Curate it while the service is stopped, then
restart.

### Run as a systemd service

A hardened unit is included at
[`etc/masterbus-signalk.service`](etc/masterbus-signalk.service):

```sh
sudo cp $(which masterbus-signalk) /usr/local/bin/             # already there if installed via cargo install
sudo cp etc/masterbus-signalk.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now masterbus-signalk
```

Nothing else needs creating by hand: systemd makes
`/etc/default/masterbus` on the first start (`ConfigurationDirectory=`
in the unit), the first run writes `config.ini` there with the
auto-detected transport and master settings, and `mapping.json` appears
next to it once devices are discovered. Review `config.ini` after the
first start; `journalctl -u masterbus-signalk` shows what was detected.

That one directory holds everything: transport, master role,
schema-cache directory and the `listen` address all live in
`config.ini` (see the **Configuration** section above), with
`mapping.json` beside it, so the unit needs no environment of its own. The service keeps a persistent
schema cache in `/var/lib/masterbus` and restarts on failure.

Upgrading from a release that used `/etc/default/masterbus-signalk/`:
copy any `LISTEN=` you set into `config.ini`'s `listen` key, then delete
the directory. Its `mapping.ini` is not carried over — the format
changed from group toggles to explicit per-field paths — and the first
run seeds a fresh `mapping.json`.

The binary lives in `/usr/local/bin` rather than `/usr/local/sbin` on
purpose: it is the same executable an unprivileged user runs from a
shell to try things out, `cargo install` puts it on the user's `PATH`
alongside `masterbus-tui`, and one location for all four tools keeps
the instructions short.

### Editing it: `masterbus-tui --mapping`

Editing JSON by hand is nobody's idea of a good time, and the TUI already
knows every device, field id and live value. `--mapping` turns it into
the editor:

    masterbus-tui --mapping

- The device list gains a count of that device's mapped fields, so an
  unmapped device is visible without opening it.
- The Monitoring tab gains a Signal K column showing where each field
  publishes.
- `+` maps the selected field. The prompt is pre-filled from the existing
  mapping, or from the built-in heuristics, and shows the conversion the
  path implies — the only moment anyone can check that `°C` is about to
  become kelvin, since the file stores no scale factor. `^N` toggles
  `invert`.
- `-` unmaps the selected field.
- `a` copies this device's mapping to every other device with the same
  article, substituting each one's own instance into the paths. Fields a
  target does not have are skipped, so a cluster master's extra fields
  are not forced onto a plain member. With ten batteries on a bus this is
  the difference between a five-minute job and an hour.
- `w` writes the file. Quitting with unsaved changes asks once.

A path whose units cannot be reconciled is refused with an explanation
rather than saved, because the sidecar would only skip it later. An
unfamiliar leaf is accepted, with a note that it will carry no unit
metadata.

Devices that are switched off or off the bus keep their entries: the file
is loaded whole and only the fields you touch are changed.

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

## `masterbus-dump`

Walks the whole bus and writes one JSON document: per device its
identity and status, per group its id and menu, per field its
channel-aware id, name, unit, visualization type, writability, range and
enum option labels, plus the live values for the monitoring menu.

    masterbus-dump [options] [output.json]

- `-o, --output <file>`: write here instead of stdout.
- `--menus <list>`: which menus to enumerate, comma-separated, or `all`.
  Default `monitoring,configuration,service`.
- `--values <mode>`: `none`, `monitoring` (default) or `all`.
- `--device <hex>`: restrict to one device address; repeatable.
- `--probe`: also flat-probe the field-index space, finding fields no
  menu lists. Slow.
- `--compact`: one-line JSON instead of pretty-printed.

Examples:

    masterbus-dump -o mybus.json                      # the usual three menus
    masterbus-dump --values all --menus all -o mybus.json
    masterbus-dump --device 286CA9 --probe            # one device, to stdout

This is the tool to run when reporting a device the Signal K sidecar
does not yet map: the dump gives someone without your hardware
everything needed to write the mapping. Field ids print in the same
three-digit hex `masterbus-set-field` accepts. Device, group and field
*names* are installer-editable strings held in device EEPROM, so a
mapping table should key on the ids and read the names as documentation.

## License

Apache-2.0.
