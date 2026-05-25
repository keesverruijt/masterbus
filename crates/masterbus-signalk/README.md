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

## Choosing what to publish (mapping.ini)

If the `MAPPING` environment variable points at a file, output is gated per
device/group. As devices are discovered the file is auto-populated and rewritten;
**edit the `true`/`false` flags while the service is stopped**, then restart.

Keys are `<instance>.<menu>[.<group>] = true|false`. A group-level line overrides
the menu-level line. New entries default **off**, except a battery's `cluster`
group, which defaults **on** — so out of the box you get the battery cluster and
nothing else:

```ini
# main-batt — groups: battery, cluster
main-batt.monitoring = false
main-batt.monitoring.cluster = true

# combimaster — groups: ac-in, ac-out, dc-in-out, general
combimaster.monitoring = false
```

To publish for instance a CombiMaster master device, set 
`combimaster.monitoring = true` (or a single group, e.g. 
`combimaster.monitoring.dc-in-out = true`) and restart.

Without `MAPPING`, every mapped field is published.

## Use from a Signal K server

Add a **data connection** of type *Signal K* over **TCP**, pointing at this host
and port (e.g. `127.0.0.1:3009`). The Signal K server connects as a client and
ingests the delta stream as if from any other provider. Note that if you run Signal K in a container you will need to change 127.0.0.1 to wherever the `masterbus-signalk` server is running.

## Run as a service

A hardened systemd unit is provided in
[`masterbus-signalk.service`](masterbus-signalk.service):

```sh
sudo cp target/<triple>/release/masterbus-signalk /usr/local/bin/
sudo cp masterbus-signalk.service /etc/systemd/system/
sudo mkdir -p /etc/default/masterbus-signalk
echo 'CAN_IFACE=can0' | sudo tee /etc/default/masterbus-signalk/config   # your interface
sudo systemctl enable --now masterbus-signalk
```

The config directory `/etc/default/masterbus-signalk/` holds `config` (the
environment: `CAN_IFACE`, `LISTEN`) and `mapping.ini` (the publish toggles, see
above), which the service creates and keeps updated. It restarts on failure
(handy on a noisy bus) and keeps a persistent schema cache in `/var/lib/masterbus`.

## License

Apache-2.0.
