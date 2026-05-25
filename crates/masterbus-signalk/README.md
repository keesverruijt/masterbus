# masterbus-signalk

A [Signal K](https://signalk.org) sidecar for Mastervolt **MasterBus**, built on
the [`masterbus`](https://crates.io/crates/masterbus) crate (Linux/SocketCAN).

It subscribes to the monitoring values of every device on the bus and serves
**Signal K deltas** — newline-delimited JSON — over **TCP**, with values converted
to SI units (e.g. °C → K, % → ratio). It listens on `0.0.0.0:3009` by default.

```sh
masterbus-signalk <can-interface> [listen-addr] [cache-dir]
# e.g. masterbus-signalk can0                       # listens on 0.0.0.0:3009
#      masterbus-signalk can0 0.0.0.0:4000 ~/.cache/masterbus
```

Sample line:

```json
{"updates":[{"$source":"masterbus","timestamp":"2026-05-25T18:00:00.000Z","values":[{"path":"electrical.batteries.main-batt-4.voltage","value":26.6}]}]}
```

The Signal K instance id is the device's name, lowercased and without its leading
class word (path-sanitized) — e.g. "BAT Main Batt 4" → `main-batt-4`.

## Use from a Signal K server

Add a **data connection** of type *Signal K* over **TCP**, pointing at this host
and port (e.g. `merrimac-pi:3009`). The Signal K server connects as a client and
ingests the delta stream as if from any other provider.

## Mapping

The MasterBus-field → Signal K-path mapping lives in `map_field` in
[`src/main.rs`](src/main.rs). This prototype covers batteries and the
CombiMaster (inverter/charger); add arms per device class to extend it.

## License

Apache-2.0.
