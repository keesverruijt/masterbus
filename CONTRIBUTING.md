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

What *is* per-model is the **Signal K sidecar** (`masterbus-signalk`). It
takes the first word of each device's name — the class code the firmware
prepends, such as `BAT`, `CMR`, `MAC`, `APR`, `MSH`, `MCU`, `CHG`, `SCM` —
and looks it up in one function, `map_field`, to decide which Signal K
path each monitoring field lands on and how to convert it to SI units.
A class that is not in that function is silently skipped: the device is
discovered, it shows up in `mapping.ini`, but nothing is published for it.

So "my MSU and CHG don't show up in Signal K" almost always means "nobody
has written the ten-to-forty lines that map that class's fields yet". That
is the contribution this guide walks you through. It is a table, not an
algorithm.

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
`masterbus-tui`, `masterbus-signalk`, `masterbus-set-field`. Run one
straight away:

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
can: open `masterbus-tui`, select the device, go to the **Monitoring**
tab, and write down (or screenshot) the device's name, and every field's
group, name, unit and a typical value. That list is the entire
specification of the change.

An example prompt that has everything it needs:

> Read CONTRIBUTING.md and crates/masterbus-tools/src/bin/masterbus-signalk.rs.
> Add Signal K support for the Mastervolt `MSH` device class (a battery
> shunt / monitor). The TUI shows these monitoring fields:
>
> group "Battery": "Battery" V (13.2), "Battery" A (-4.5),
> "State of charge" % (87), "Time remaining" (a Time value),
> "Battery" °C (21)
>
> group "Shunt": "Consumed" Ah (-32)
>
> Map them onto `electrical.batteries.<id>` like the existing `BAT`
> class does. Add tests in the same style as the existing ones, then run
> `make precommit` and fix anything it reports.

Then **check the result yourself**, which needs no Rust: build, run
`masterbus-signalk`, and look at the stream with `nc localhost 3009`
(or a Signal K server). Do the values match what the TUI shows, in SI
units? Volts stay volts, but temperatures must be Kelvin, percentages
ratios 0..1, rpm becomes Hz. If a value is wrong, tell the AI what you
saw and what you expected. If it claims the tests pass, run `make
precommit` yourself and look.

Ask the AI to explain any change you do not understand before you send
it in. You are the one signing the pull request.

## 6. Anatomy of a device-class addition

So you can follow what the AI does (or do it by hand), here is what a
class needs. Everything is in
`crates/masterbus-tools/src/bin/masterbus-signalk.rs`.

1. **`sk_bases`** — the Signal K node(s) a class publishes under, used
   for the static `name` / `manufacturer` metadata. One line:
   `"MSH" => vec![format!("electrical.batteries.{id}")],`

2. **`map_field`** — a `match` arm per class. Inside, a `match` on
   `(name, unit)` pairs — the *exact* strings the device reports — that
   returns a Signal K path and the converted value. Copy the arm of the
   most similar existing class (`BAT` for anything battery-like, `MAC`
   or `CMR` for chargers and inverters, `APR` for alternators) and edit
   the names. Fields you leave out are simply not published; that is
   fine for a first version.

3. **`sk_units`** — only if you introduce a new leaf name (the last
   path segment). Existing leaves such as `voltage`, `current`,
   `temperature`, `stateOfCharge` already carry units.

4. **Tests** at the bottom of the file, in the `#[cfg(test)]` module.
   The existing ones show the style: call `map_field` with a class, a
   name, a unit and a value, assert the path and the converted number.

5. A line in [CHANGELOG.md](CHANGELOG.md) under `[Unreleased]`.

Pick Signal K paths from the [Signal K specification](https://signalk.org/specification/1.7.0/doc/vesselsBranch.html)
where a standard one exists (`electrical.batteries`, `electrical.chargers`,
`electrical.inverters`, `electrical.alternators`, `electrical.solar`).
When Signal K has no standard leaf for a field, nest it under the device
node with a descriptive camelCase name, the way `APR` does with
`.battery.voltage` and `.engine.revolutions`.

Which fields to map: values a dashboard would want — voltages, currents,
power, state of charge, temperatures, charger state, on/off. Skip
configuration knobs; the sidecar only publishes the Monitoring menu.

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

Not up for a PR at all? Open an issue with the field list from section 5
(device name, and every monitoring field's group, name, unit and a sample
value). That is enough for someone else to write the mapping blind.

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
| `crates/masterbus-tools/src/bin/masterbus-signalk.rs` | the Signal K sidecar, including the per-class mapping |
| `crates/masterbus-tools/src/bin/masterbus-set-field.rs` | one-shot field writer |
| `crates/masterbus-tools/etc/` | the systemd unit |
| `crates/masterbus-ffi/` | C ABI wrapper and C demos |
| `docs/PROTOCOL.md` | the wire protocol, as reverse engineered |
