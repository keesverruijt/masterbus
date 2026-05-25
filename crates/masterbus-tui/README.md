# masterbus-tui

A terminal UI for browsing and editing Mastervolt **MasterBus** devices, built on
the [`masterbus`](https://crates.io/crates/masterbus) crate (Linux/SocketCAN).

```sh
masterbus-tui <can-interface> [cache-dir]   # e.g. masterbus-tui can0 ~/.cache/masterbus
```

Devices are listed on the left with liveness; the selected device's groups and
fields are on the right. `Tab` / `Shift-Tab` switch between the Monitoring /
Config / Service tabs (each discovered on demand). Writable fields are editable:
booleans toggle, numbers open an editor, lists cycle with the arrow keys; `q`
quits.

See the [repository](https://github.com/keesverruijt/masterbus) for the full
workspace.

## License

Apache-2.0.
