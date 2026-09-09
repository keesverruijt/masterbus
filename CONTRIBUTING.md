# Contributing — you do not need to know Rust

This file is for people who want to improve the project itself.

**If you just want your own MasterBus devices in Signal K, you are in the
wrong file.** That needs no build, no Rust and nothing from here: download
a release, run `masterbus-tui --mapping`, and map your devices. See
[ENDUSER.md](ENDUSER.md), with [HARDWARE.md](HARDWARE.md) for connecting
to the bus.

What is left, once every installation curates its own mapping, is the
software underneath: the transports, the protocol decoding, discovery, the
terminal UI, the C ABI. Section 3 is the map; section 4 is where the work
is.

## 1. Install git and the Rust toolchain

Everything from here on needs a toolchain. You need two things: **git**, to fetch the code and send changes back,
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

## 2. Get the code and build it

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

## 3. Where things live

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
## 4. Working on the library

Most of what there is to build is in `crates/masterbus`. The tools on top
are deliberately thin — the TUI browses whatever discovery found, and the
sidecar publishes whatever a file names — so the interesting problems are
underneath them.

**A value that decodes wrong** is almost always in
`src/protocol/decode.rs`, which turns a frame's bytes into a typed
`Value`, paired with `encode.rs` on the write path. Both have unit tests
beside them and neither needs hardware to work on: a failing case is a
byte array and an expected value.

**A device that never finishes enumerating**, or whose menus come back
wrong, is `src/runtime/discovery.rs`. It walks group counts, then groups,
then per-field metadata, with a schema cache on disk so the second run is
fast. This is where most of the protocol's surprises live, and
[docs/PROTOCOL.md](docs/PROTOCOL.md) records the ones already understood.

**Bus plumbing** is `src/transport/` (SocketCAN and the USB link) and
`src/runtime/` (the reader thread, the scheduler that paces requests, the
value cache). `src/runtime/framelog.rs` is the candump-style trace.

**The protocol document is part of the deliverable.** Anything you work
out about the wire belongs in `docs/PROTOCOL.md` in the same PR. It was
reverse engineered from traces and it is the reason the next person does
not have to start over.

If you want a defined task rather than a bug, the **TODO** list at the
bottom of the [README](README.md) is honest about what is missing.

### Getting a trace

Whatever you are chasing, start by watching the wire:

```sh
RUST_LOG=masterbus=debug,masterbus::frame=trace \
    ./target/release/masterbus-tui 2> trace.log
```

Reproduce the problem, quit, and read `trace.log`. The `masterbus::frame`
target is every frame sent and received, in a candump-compatible format,
with a tag saying whether it was a read, a write or a schema query.

**If you would rather not chase it yourself**, that trace is exactly what
someone else needs. Attach it to an issue with the device's article number
and firmware version from the TUI's Summary tab. Nothing in it is secret
beyond your devices' serial numbers.

