# masterbus

An idiomatic Rust client for the Mastervolt **MasterBus** CAN-bus protocol, with
a terminal UI and a C-ABI wrapper.

A Cargo workspace:

| crate | what |
|-------|------|
| [`masterbus`](crates/masterbus) | core library (the protocol, discovery, caching, subscriptions, the `MasterBus`/`Device`/`Tab`/`Field` API and a non-blocking channel API) |
| [`masterbus-ffi`](crates/masterbus-ffi) | C ABI `cdylib` (single-threaded), header generated with cbindgen |
| [`masterbus-tui`](crates/masterbus-tui) | terminal UI to browse devices/values and edit settings |

The wire protocol is documented in [`docs/PROTOCOL.md`](docs/PROTOCOL.md).

## Status

Early development — built up incrementally. See the crate docs for what's wired
up so far.

## Design highlights

- **Fast init**: `connect()` returns as soon as the bus is usable and one device
  is heard — no multi-second enumeration wait, so one-shot tools stay snappy.
- **Lazy, single-threaded discovery** with an optional on-disk schema cache
  (keyed by article + firmware) — long-running programs discover once.
- **Live values without polling storms**: a passive value cache fed by bus
  traffic, plus rate-based subscriptions that actively poll only what's needed,
  paced to a bus budget.
- **Two API surfaces over one core**: a blocking navigator API for simple
  one-shots, and a non-blocking channel/event API for the TUI and daemons.

## License

MIT OR Apache-2.0
