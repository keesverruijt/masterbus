# Contributing — you do not need to know Rust

This file is for people who want to improve the project itself. An AI
assistant can do the typing; section 5 is about that.

**If you just want your own MasterBus devices in Signal K, you are in the
wrong file.** That needs no build, no Rust and nothing from here: download
a release, run `masterbus-tui --mapping`, and map your devices. See
[ENDUSER.md](ENDUSER.md), with [HARDWARE.md](HARDWARE.md) for connecting
to the bus. You do not have to change this project's code and you do not
have to wait for anyone.

What is left, once every installation curates its own mapping, is the
software underneath: the transports, the protocol decoding, discovery, the
terminal UI, the C ABI. That is section 4, and it is where the real work
is. The bundled *guesses* a fresh mapping is seeded from are worth
improving too, but they are a table of data rather than an engineering
problem — section 6, and the smaller job.

Section 3 is a map of the codebase. Read it before either.

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

## 5. Let an AI do the typing

The maintainer wrote most of this project with Claude Code, and it suits
the work: bounded problems, a documented protocol, tests that do not need
hardware. Any agentic coding tool works — Claude Code, Codex, Cursor,
Copilot's agent mode, Aider. Install one, open it in the cloned
`masterbus` directory, and talk to it.

Three things make this go well:

**Give it the right context.** Point it at this file, at
[README.md](README.md), and at the code it will change. For anything
touching the wire, [docs/PROTOCOL.md](docs/PROTOCOL.md) is the reference
and is written to be read by an AI as much as by a human. Section 3 says
what lives where; paste the relevant row.

**Give it evidence, not a description.** A frame trace for a protocol
problem, a failing byte sequence for a decode bug, a `masterbus-dump` for
anything about a specific device. "The voltage looks wrong" is not
something an assistant can act on; forty lines of `trace.log` is.

**Make it prove the change.** Every crate has tests beside the code and
`make precommit` runs the lot, plus rustfmt, clippy and the doc build. Ask
for a failing test first where that is possible. If it claims the tests
pass, run `make precommit` yourself and look.

The AI cannot see your bus. One command gives it everything about a
device:

```sh
./target/release/masterbus-dump -o mybus.json
```

That is every device, group and field — id, name, unit, range, enum
options — plus live values and your own mapping, as JSON.

An example prompt that has everything it needs:

> Read CONTRIBUTING.md and
> crates/masterbus-tools/src/suggestions/catalog.json. Add an entry for
> Mastervolt article 66025000 (an MLI Ultra battery), firmware 1.37, from
> the attached mybus.json and mapping.json. Use the field ids, not the
> field names — the names differ between models. Set `source` to say the
> entry came from one boat's dump. Then run `make precommit` and fix
> anything it reports.

If the model really does belong to a class whose field *names* are
consistent, say so and point it at `seed.rs` instead, asking for tests in
the style of the ones already there.

Then **check the result yourself**, which needs no Rust. Do not delete
your `mapping.json` to do it — that is the file you curated for your own
boat.
Point the program at a scratch copy instead, so a fresh one gets seeded
somewhere harmless:

```sh
MAPPING=/tmp/try.json ./target/release/masterbus-signalk 0.0.0.0:3010
```

With `masterbus-signalk` stopped, that seeds `/tmp/try.json` from the
guesses your change just edited and streams the result on a spare port.
Look at it with `nc localhost 3010`, or open `/tmp/try.json` and read the
paths. Do the values match what the TUI shows, in SI units? Volts stay
volts, but temperatures must be kelvin, percentages ratios 0..1, rpm
becomes Hz. If a value is wrong, tell the AI what you saw and what you
expected. If it claims the tests pass, run `make precommit` yourself and
look.

Ask the AI to explain any change you do not understand before you send
it in. You are the one signing the pull request.

## 6. Teaching the shipped guesses

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
   git checkout -b decode-devicelist-index
   ```
3. Make the change, run `make precommit` until it is green.
4. Commit and push:
   ```sh
   git add -A
   git commit -m "protocol: decode DeviceList from the f32 index"
   git push fork decode-devicelist-index
   ```
5. GitHub shows a banner offering to open a **pull request**. Do that. In
   the description, say what you tested against — the device's article
   number and firmware version from the TUI's Summary tab if it is
   hardware-specific — and paste the evidence: the trace line that
   changed, the values before and after, or the Signal K output.

Your AI assistant can do steps 2 through 4 for you if you ask; the `gh`
command-line tool can even open the PR. Small PRs, one topic at a time,
are easier to review than one PR for five.

Not up for a PR at all? Open an issue and attach the `mybus.json` from
section 5. It carries your mapping alongside the bus, so it is enough for
someone else to turn your work into a bundled suggestion without your
hardware. It contains your devices' names, serial numbers and current
readings — nothing secret, but if you would rather not publish serial
numbers, edit them out first, or use `--device <address>` to dump only
the one device.

