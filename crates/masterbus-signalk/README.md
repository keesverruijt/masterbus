# masterbus-signalk

A [Signal K](https://signalk.org) sidecar for Mastervolt **MasterBus**, built on
the [`masterbus`](https://crates.io/crates/masterbus) crate (Linux/SocketCAN).

It subscribes to the monitoring values of every device on the bus and prints
**Signal K deltas** — one JSON object per line — to stdout, with values converted
to SI units (e.g. °C → K, % → ratio).

```sh
masterbus-signalk <can-interface> [cache-dir]
# e.g. masterbus-signalk can0 ~/.cache/masterbus
```

Sample line:

```json
{"updates":[{"$source":"masterbus","timestamp":"2026-05-25T18:00:00.000Z","values":[{"path":"electrical.batteries.Main-Batt-4.voltage","value":26.6}]}]}
```

The Signal K instance id is the device's name without its leading class word
(path-sanitized) — e.g. "BAT Main Batt 4" → `Main-Batt-4`.

## Use from a Signal K server

Add a **data connection** of type *Execute* with the Signal K (delta) data
format, running this binary against your CAN interface — the server ingests the
deltas as if from any other provider.

## Mapping

The MasterBus-field → Signal K-path mapping lives in `map_field` in
[`src/main.rs`](src/main.rs). This prototype covers batteries and the
CombiMaster (inverter/charger); add arms per device class to extend it.

## License

Apache-2.0.
