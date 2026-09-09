# For end users

Let's say you have a MasterBus network (at least one device, probably more)
and you want to integrate your MasterBus data — battery State Of Charge,
current draw, inverter status — into some other software. But you're NOT
a programmer, and you thought Rust was what happens when iron oxidizes.

How can this project help? It's not *so* hard.

## What you need

A computer that can reach the bus. Two options, both covered in
[HARDWARE.md](HARDWARE.md):

- A **Mastervolt USB Interface** (article 77030200, ~€200) — works on
  Linux, macOS, and Windows. Plug-and-play, no fiddling.
- A **Linux machine with a cheap CAN adapter** — a CANable USB stick
  (~€25), a PiCAN HAT on a Raspberry Pi, or any SocketCAN-capable port.
  Bring up `can0` at 250 kbit/s and you're set.

Then download the latest binary release from
<https://github.com/keesverruijt/masterbus/releases> — there is a tarball
per platform (Linux x86_64 / armv7 / aarch64, macOS Intel / Apple Silicon,
Windows). Every tarball carries `masterbus-tui`, `masterbus-set-field`
and `masterbus-dump`, plus `masterbus-signalk` and its systemd unit on
Linux. Or build from source: `cargo build --release` produces the same
binaries under `target/release/`; [CONTRIBUTING.md](CONTRIBUTING.md)
walks through installing the toolchain.

## First steps: explore the bus

Run `masterbus-tui` to see what's on your bus. It's a terminal
application that lists every device, lets you drill into its menus
(Summary, Monitoring, Configuration, Service, Settings), and shows live
values. Writable fields can be edited inline.

    masterbus-tui

The first run creates a config file with the detected transport (USB 
link if present, otherwise the lone CAN interface) or fails with
an error if there are multiple CAN devices. 
Edit that file to switch transports or enable bus-master
mode; see the **Configuration** section in the project README.

You'll see a left pane with all alive devices and a right pane with
that device's tabs. `Tab` / `Shift+Tab` switch tabs, arrow keys move
between fields, `Enter` opens an editor on writable fields, `l` opens
the access-level (login) modal. Higher access levels unlock more
fields — for many configuration changes you need at least Installer.

The first time you connect to a device it takes a few seconds to
discover its schema; afterwards everything is cached on disk and the
TUI feels instant.

## Reading data continuously

For pulling data off the bus in a long-running stream, use
`masterbus-signalk`. It connects to the bus and emits Signal K deltas
(newline-delimited JSON) on a TCP socket.

    masterbus-signalk

Then `nc localhost 3009` shows the live stream. Any language that can
read a TCP socket and parse JSON can consume this — Python, Node,
shell, anything. Despite the name it works fine without a Signal K
server: it's just JSON lines.

It does **not** publish everything it finds. What goes out is a list you
control, one entry per field, and the next section is about building it.
Read that before you conclude the thing is broken.

## Telling it what to publish

**You have to do this.** Nothing else in this guide matters if you skip
it: a device that is not in your mapping produces no Signal K data, no
matter how happily it shows up in the TUI.

    masterbus-tui --mapping

That is the same browser as before with an editor attached. The device
list gains a count of how many of each device's fields are published, so
the ones producing nothing are obvious. Open a device, go to the
**Monitoring** tab, and:

- `+` publishes the selected field. A box opens with the Signal K path
  filled in where the program has a good guess, and shows the unit
  conversion it will apply — this is your chance to notice that °C is
  about to become kelvin, which is what Signal K wants.
- `-` stops publishing it.
- `a` copies this device's whole setup to every other device of the same
  model. Ten identical batteries become one minute of work instead of
  ten.
- `w` saves. Stop `masterbus-signalk` before you edit, and start it
  again afterwards.

Paths are yours to choose. If a charger really feeds the bow thruster
bank, call it that. Pick names from the
[Signal K specification](https://signalk.org/specification/1.7.0/doc/vesselsBranch.html)
where one fits (`electrical.batteries`, `electrical.chargers`,
`electrical.inverters`, `electrical.alternators`, `electrical.solar`).

A path whose units make no sense for it is refused with an explanation
rather than saved. An unfamiliar name is accepted, with a note that it
will publish without unit information.

What you are editing is `mapping.json`, next to `config.ini` (see the
**Configuration** table in the [README](README.md) for where that is on
your platform). You never have to open it, but it is plain text if you
want to, and the
[tools README](crates/masterbus-tools/README.md#what-gets-published-mappingjson)
describes the format. Back it up once you have it the way you want it.

### Why it isn't automatic

The first time `masterbus-signalk` runs with no mapping it guesses one
from a built-in list of common models and field names, so a fresh
install is not silent. Treat that as a starting point, not an answer. On
a real fourteen-device installation the guesses covered three devices;
the other eleven needed a human. Mastervolt gives two completely
different chargers the same class code, installers rename fields freely,
and two models of the same battery call their voltage different things.
Guessing wrong and publishing it anyway would put bad numbers on your
dashboard, so the program guesses, shows you, and waits.

## Writing values

For one-off changes (renaming switches, toggling an inverter, setting
a charge profile), `masterbus-tui` is the tool — navigate to the field,
press Enter, edit, Enter again to commit. Text fields, dropdowns,
booleans and numerics all round-trip.

For programmatic / scripted writes (e.g. switching a charger on from a
shell script or cron), the API is exposed through the
[`masterbus`](crates/masterbus) Rust crate. A few lines of Rust:

```rust
use masterbus::{Config, MasterBus, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Picks the transport from the per-host config file (auto-created on
    // first run; see `masterbus::FileConfig`).
    let bus = MasterBus::auto(Config::default())?;
    let device = bus.device(0x188EA2);          // your device id
    let field  = device.field(0x0013);          // your field id (three-digit hex from the TUI)
    field.set(Value::Boolean(true))?;            // turn it on
    Ok(())
}
```

For shell-script / one-off writes there's also a dedicated CLI:

    masterbus-set-field <device_id> <field_id> <value>

- `<device_id>` is the hex address from the TUI's title bar, e.g.
  `188EA2`.
- `<field_id>` is the three-digit hex shown next to each editable row in
  the TUI, e.g. `0x013` for a Btm1 field, `0x10E` for a Btm3 one (bit 8
  selects the channel).
- `<value>` is parsed against the field's type:
    - **boolean**: `true` / `false` / `on` / `off` / `1` / `0`
    - **number**: any decimal number
    - **list**: either the option index (`2`) or the exact label
      (`"Stabilized"`)
    - **text**: free string, capped at 16 printable-ASCII chars (the
      device-side limit; the TUI enforces the same cap on its editor)

Examples:

    masterbus-set-field 188EA2 0x013 on             # toggle a CombiMaster bool
    masterbus-set-field 3A3B4B 0x104 "Nav Chg"      # rename device name
    masterbus-set-field 53A493 0x160 "New Name"     # rename EasyView Switch 1

The TUI shows the *device id* (title bar, e.g. `[188EA2]`) and the
*field id* on every editable row, so picking the right ids is a
copy-paste away.

## Helping the next person

Once you have mapped a device, the guesses can be taught to cover it, so
the next owner of that model gets it filled in for free. That needs one
file from you:

    masterbus-dump --values all --menus all -o mybus.json

It records every device, group and field with its id, name, unit, range
and current value, **and your mapping alongside it** — the bus and what
you decided it means, together. Attach it to an issue. If the bus is
large, `--device <id>` limits the dump to the one device you care about.

The file contains your devices' names, serial numbers and whatever they
were reading at the time. Nothing secret, but edit it first if you would
rather not publish serial numbers.

The same file is what to attach if a device misbehaves rather than
merely being unmapped.
