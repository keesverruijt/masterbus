# Contributing — even if you are not a developer

This file is for the boat owner who has a MasterBus network with devices
this project does not (fully) understand yet, and who wants to help fix
that. Maybe you wrote software twenty years ago, maybe never. You do not
need to know Rust. You need a computer that can reach the bus, an hour
to set up a toolchain, and — optionally but highly recommended — an AI
coding assistant to do the typing.

If you *are* a developer: the short version is `make precommit` before you
open a PR, and the rest of this file will still tell you where things live.

## 1. First: do you even need to build anything?

Probably not for *using* the project. Prebuilt binaries for every release
are attached to <https://github.com/keesverruijt/masterbus/releases> —
Linux (x86_64, armv7, aarch64 — the last two cover every Raspberry Pi),
macOS (Intel and Apple Silicon) and Windows. Unpack the tarball for your
platform and run `masterbus-tui`. See [ENDUSER.md](ENDUSER.md) for that
path and [HARDWARE.md](HARDWARE.md) for how to connect to the bus.

You need to build from source when you want to *change* something:
typically add a device class to the Signal K sidecar, or fix a field that
decodes wrong. Read on.

## 2. What "not supported" usually means

This is the single most important thing to understand before you start,
because it decides how much work you are in for.

The core library discovers **every** device on the bus generically. It
does not have a list of known models. If a device announces itself, the
TUI (`masterbus-tui`) lists it and lets you browse all its menus and live
values, whatever it is. There is no per-model code to add for that.

What the **Signal K sidecar** (`masterbus-signalk`) publishes is decided
by a file, not by code: `mapping.json`, beside `config.ini`. It says which
field of which device publishes to which Signal K path, keyed on the
device's serial number and the field's id. Anything not listed there is
not published.

So "my MSU and CHG don't show up in Signal K" means "nothing in your
mapping file points at them yet", and the fix is on your own boat, in
your own file. You do not need to change this project's code, and you do
not have to wait for anyone.

On the first run with no mapping file, the sidecar seeds one from
built-in per-class name heuristics, so common devices arrive already
mapped. Those heuristics are a *guess*: they match on the first word of
the device name (`BAT`, `CMR`, `MAC`, `APR`) and on field names, and real
bus surveys show both of those vary between models and get renamed by
installers. A guess that misses simply leaves a field out of the seed for
you to add.

That gives two different contributions, and it is worth knowing which one
you are making. Editing your own `mapping.json` fixes your boat today.
Improving the seed heuristics (section 6) makes the next person's file
start closer to right. The second is optional and strictly a bonus.

The other, rarer case is a device that misbehaves in the TUI itself (a
field with a nonsense value, a menu that never finishes discovering). That
is a protocol issue and section 8 tells you how to capture what the
maintainer needs.

## 3. Install git and the Rust toolchain

You need two things: **git**, to fetch the code and send changes back,
and the Rust toolchain. Rust installs with one tool, `rustup`, which
manages the compiler (`rustc`), the build tool and package manager
(`cargo`), and updates. Everything below is a one-time setup.

### Linux, including Raspberry Pi

```sh
sudo apt install git build-essential pkg-config     # Debian / Ubuntu / Raspberry Pi OS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

The first line installs git plus the C compiler and linker Rust needs.
On Fedora it is `sudo dnf install git gcc`, on Arch `sudo pacman -S git
base-devel`. Check with `git --version`.

For the rustup line, accept the defaults. Then either open a new shell or run
`source "$HOME/.cargo/env"`. Check with `cargo --version`.

You can build directly on a Pi 4 or 5. The first release build takes a
coffee break (the project uses link-time optimisation); later builds only
recompile what you changed and are much faster. A `cargo build` without
`--release` is quicker still and fine for testing.

### macOS

```sh
xcode-select --install       # Apple's command-line tools: git, compiler, linker
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

The first command pops up a dialog; confirm it and wait for the download.
It includes git, so there is nothing separate to install. (If you use
Homebrew, `brew install git` gives a newer git, but the Apple one is
fine.) Check with `git --version`.

The Mastervolt USB link works on macOS; SocketCAN does not exist there.

### Windows

Install **Git for Windows** from <https://git-scm.com/download/win>.
Accept the defaults; the only choice that matters is to keep "Git from
the command line and also from 3rd-party software" so `git` works in
PowerShell. (Alternatively `winget install Git.Git` from a PowerShell
window does the same.) Open a new PowerShell window afterwards and check
with `git --version`.

Then download and run `rustup-init.exe` from <https://rustup.rs>. Rust on
Windows needs Microsoft's C++ linker; the installer notices when it is
missing and offers to install the **Visual Studio Build Tools** for you
("Quick install via the Visual Studio Community installer"). Say yes. It
is a large download and it is the least pleasant part of the whole
process, but it is automatic.

Use the Mastervolt USB link; SocketCAN is Linux-only. Run the commands
below from a regular PowerShell or "Command Prompt" window, not from WSL,
because WSL cannot see the USB HID device.

### Minimum version

The project needs Rust 1.85 or later (`rust-version` in `Cargo.toml`).
`rustup update` brings you to the current stable release.

## 4. Get the code and build it

```sh
git clone https://github.com/keesverruijt/masterbus.git
cd masterbus
cargo build --release
```

That builds everything. The binaries land in `target/release/`:
`masterbus-tui`, `masterbus-signalk`, `masterbus-set-field`,
`masterbus-dump`. Run one straight away:

```sh
./target/release/masterbus-tui
```

The first run auto-creates a config file with the detected transport
(see **Configuration** in the [README](README.md)).

The cheat sheet:

| Command | What it does |
|---------|--------------|
| `cargo build` | debug build (fast to compile, slower to run) |
| `cargo build --release` | optimised build, what you deploy |
| `cargo run --release --bin masterbus-tui` | build if needed, then run |
| `cargo test --workspace` | run every unit test |
| `make precommit` | format check, lints, tests — what CI runs on a PR |
| `make fmt` | reformat your code so the format check passes |

If `make` is not installed, the Makefile header lists the `cargo`
commands each target expands to.

## 5. Let an AI do the typing

The maintainer wrote most of this project with Claude Code, and adding a
device class is exactly the kind of bounded, well-specified task an AI
assistant is good at. Any agentic coding tool works: Claude Code, Codex,
Cursor, Copilot's agent mode, Aider. Install one, open it in the cloned
`masterbus` directory, and talk to it.

Two things make this go well:

**Give it the right context.** Point it at this file, at
[README.md](README.md), and at the file it will edit
(`crates/masterbus-tools/src/bin/masterbus-signalk.rs`). For anything
touching the wire protocol, [docs/PROTOCOL.md](docs/PROTOCOL.md). These
are written to be read by an AI as much as by a human.

**Give it the facts from your bus.** The AI cannot see your devices. You
can. The quickest way is one command:

```sh
./target/release/masterbus-dump -o mybus.json
```

That writes every device, group and field — id, name, unit, range, enum
options — plus the live monitoring values, as JSON. Point the AI at the
file. Failing that, open `masterbus-tui`, select the device, go to the
**Monitoring** tab and screenshot it; the field id in the left column is
the part that matters most.

An example prompt that has everything it needs:

> Read CONTRIBUTING.md and crates/masterbus-tools/src/seed.rs.
> Add seed suggestions for the Mastervolt `MSH` device class (a battery
> shunt / monitor). The TUI shows these monitoring fields:
>
> group "Battery": "Battery" V (13.2), "Battery" A (-4.5),
> "State of charge" % (87), "Time remaining" (a Time value),
> "Battery" °C (21)
>
> group "Shunt": "Consumed" Ah (-32)
>
> Map them onto `electrical.batteries.<instance>` like the existing `BAT`
> class does. Add tests in the same style as the existing ones, then run
> `make precommit` and fix anything it reports.

Then **check the result yourself**, which needs no Rust: delete your
`mapping.json` so it is seeded afresh, run `masterbus-signalk`, and look
at the stream with `nc localhost 3009` (or a Signal K server). Do the
values match what the TUI shows, in SI units? Volts stay volts, but
temperatures must be Kelvin, percentages ratios 0..1, rpm becomes Hz. If
a value is wrong, tell the AI what you saw and what you expected. If it
claims the tests pass, run `make precommit` yourself and look.

Ask the AI to explain any change you do not understand before you send
it in. You are the one signing the pull request.

## 6. Two ways to fix an unmapped device

### The one that fixes your boat: `masterbus-tui --mapping`

You do not have to write JSON. Stop the service, then:

```sh
masterbus-tui --mapping
```

The device list shows how many of each device's fields publish, so an
unmapped device stands out. Open one, go to the Monitoring tab, and press
`+` on a field. The prompt arrives pre-filled with a suggestion where the
built-in heuristics have one, and shows the unit conversion the path
implies, which is the moment to check that °C is about to become kelvin.
`-` unmaps. `a` copies the whole device's mapping to every other device
with the same article, which is what makes ten identical batteries a
one-minute job. `w` writes the file.

A path whose units cannot be reconciled is refused with an explanation.
An unfamiliar leaf is accepted, with a note that it will publish without
unit metadata.

### The same thing by hand

The file sits beside `config.ini` (`/etc/default/masterbus/` on a Linux
system install; see the **Configuration** table in the README for the
other platforms).

```json
{
  "version": 1,
  "devices": {
    "1937R08110": {
      "article": "40021006",
      "firmware": "7.9",
      "name": "CHG 24V Ch.U4-1",
      "instance": "24v-ch-u4-1",
      "fields": {
        "0x00E": { "path": "electrical.chargers.24v-ch-u4-1.voltage" },
        "0x00F": { "path": "electrical.chargers.24v-ch-u4-1.current" },
        "0x011": { "path": "electrical.chargers.24v-ch-u4-1.temperature" }
      }
    }
  }
}
```

Everything you need is in the TUI: the serial on the device's Summary
tab, and the field id in the left column of every Monitoring row.
`masterbus-dump` gives you the same thing as one file.

Three things to know:

- **Presence is the toggle.** A field you do not list is not published.
- **You never write a scale factor.** The conversion to SI follows from
  the field's unit and the unit the path's last segment implies, so `°C`
  into a `temperature` leaf becomes kelvin by itself. A pair that cannot
  be reconciled is reported at startup and skipped, so a mistake tells
  you rather than publishing a wrong number.
- **The path is yours.** A non-standard leaf or a different category is
  honoured, and the device's `name` metadata follows it there. You will
  get a warning that an unknown leaf carries no unit metadata.

Pick paths from the [Signal K specification](https://signalk.org/specification/1.7.0/doc/vesselsBranch.html)
where a standard one exists (`electrical.batteries`, `electrical.chargers`,
`electrical.inverters`, `electrical.alternators`, `electrical.solar`).
Where Signal K has no standard leaf, nest it under the device node with a
descriptive camelCase name, the way the `APR` seed does with
`.battery.voltage` and `.engine.revolutions`.

### The one that helps everyone: teach the suggestions

Optional, and only worth doing for a model several boats will have.
Suggestions come in two tiers and you want the right one.

**A specific model → the bundled database.** Add an entry to
`crates/masterbus-tools/src/suggestions/catalog.json`, keyed on the
device's article number and field ids, with `{instance}` standing in for
the Signal K instance:

```json
"40021006": {
  "model": "Mastervolt Mass Charger (single output)",
  "source": "issue #6, 28-device dump, firmware 7.9",
  "fields": {
    "0x00E": { "path": "electrical.chargers.{instance}.voltage" },
    "0x00F": { "path": "electrical.chargers.{instance}.current" }
  }
}
```

This is the better tier and usually the right one. It cannot be confused
by a rename, and it tells apart models that share a class code — two
charger articles both call themselves `CHG` with completely different
field sets. A `masterbus-dump` from the device gives you everything you
need. Fill in `source` honestly: "one boat reported this" is different
evidence from "the vendor documents it", and that difference should
survive into review.

**A whole class, by field name → the heuristics.** Only where the names
are genuinely consistent across models. It is one file,
`crates/masterbus-tools/src/seed.rs`.

1. **`suggest`** — a `match` arm per class. Inside, a `match` on
   `(name, unit)` pairs, the *exact* strings the device reports, each
   returning a Signal K path. Copy the arm of the most similar existing
   class (`BAT` for anything battery-like, `MAC` or `CMR` for chargers
   and inverters, `APR` for alternators) and edit the names. A class
   whose devices are named inconsistently across models can list both
   spellings, as `BAT` does for `Battery` and `Voltage`.

2. **`signalk::leaf_unit`** in `src/signalk.rs` — only if you introduce a
   new leaf name. Existing leaves such as `voltage`, `current`,
   `temperature` and `stateOfCharge` already carry units. If your leaf is
   a new physical quantity, `units::conversion` may need a row too.

3. **Tests** at the bottom of `seed.rs`. The existing ones show the
   style. One of them checks that *every* path the table proposes has a
   derivable conversion; a new suggestion that fails it is pointing at a
   leaf whose unit nothing can reach.

4. A line in [CHANGELOG.md](CHANGELOG.md) under `[Unreleased]`.

Either way, remember what a suggestion is for: it is a starting point a human then edits,
never the last word. Suggesting nothing is always better than suggesting
something wrong.

## 7. Sending it back

You need a free GitHub account.

1. On the repository page click **Fork**. That gives you your own copy.
2. Point your clone at it (or clone the fork instead), and make a branch:
   ```sh
   git remote add fork git@github.com:<you>/masterbus.git
   git checkout -b msh-signalk
   ```
3. Make the change, run `make precommit` until it is green.
4. Commit and push:
   ```sh
   git add -A
   git commit -m "signalk: add MSH battery-shunt class"
   git push fork msh-signalk
   ```
5. GitHub shows a banner offering to open a **pull request**. Do that. In
   the description, say which device (article number and firmware
   version from the TUI's Summary tab) you tested against and paste a
   few lines of the resulting Signal K output.

Your AI assistant can do steps 2 through 4 for you if you ask; the `gh`
command-line tool can even open the PR. Small, one-class PRs are easier
to review than one PR for five classes.

Not up for a PR at all? Open an issue and attach the `mybus.json` from
section 5. It carries your mapping alongside the bus, so it is enough to
turn your work into a bundled suggestion for the next person. That is enough for someone else to write the mapping blind.
It contains your devices' names, serial numbers and current readings —
nothing secret, but if you would rather not publish serial numbers, edit
them out first, or use `--device <address>` to dump only the one device.

## 8. When the device itself misbehaves

If a device is missing from the TUI, a value looks like garbage, or
discovery of a menu never completes, the fix is in the core library and
needs a trace of the actual bus traffic. Capture it like this:

```sh
RUST_LOG=masterbus=debug,masterbus::frame=trace \
    ./target/release/masterbus-tui 2> trace.log
```

Reproduce the problem (select the device, open the offending tab), quit,
and attach `trace.log` to an issue together with the device's article
number and firmware version. The `masterbus::frame` target is a
candump-style dump of every frame sent and received, which is what the
protocol notes in [docs/PROTOCOL.md](docs/PROTOCOL.md) were reverse
engineered from. Nothing in it is secret beyond your devices' serial
numbers.

## 9. Where things live

| Path | What |
|------|------|
| `crates/masterbus/` | the library: transports, protocol, discovery, value cache, the `MasterBus`/`Device`/`Group`/`Field` API |
| `crates/masterbus/src/protocol/` | frame encoding and decoding |
| `crates/masterbus/src/runtime/discovery.rs` | how a device's menus, groups and fields are enumerated |
| `crates/masterbus/src/strings/catalog.json` | bundled string tables that make discovery fast for known firmware images |
| `crates/masterbus-tools/src/bin/masterbus-tui/` | the terminal UI |
| `crates/masterbus-tools/src/bin/masterbus-signalk.rs` | the Signal K sidecar |
| `crates/masterbus-tools/src/mapping.rs` | the `mapping.json` format |
| `crates/masterbus-tools/src/seed.rs` | per-class path suggestions used to seed a new mapping |
| `crates/masterbus-tools/src/database.rs` | per-model path suggestions, keyed on article number |
| `crates/masterbus-tools/src/suggestions/catalog.json` | the bundled per-model data |
| `crates/masterbus-tools/src/units.rs` | device-unit → SI conversion, derived from the unit pair |
| `crates/masterbus-tools/src/bin/masterbus-set-field.rs` | one-shot field writer |
| `crates/masterbus-tools/src/bin/masterbus-dump.rs` | whole-bus JSON snapshot |
| `crates/masterbus-tools/etc/` | the systemd unit |
| `crates/masterbus-ffi/` | C ABI wrapper and C demos |
| `docs/PROTOCOL.md` | the wire protocol, as reverse engineered |
