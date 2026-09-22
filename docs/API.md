# The `masterbus-signalk` control API

`masterbus-signalk` serves two things. The **delta stream** (newline-delimited
Signal K deltas over TCP, default `0.0.0.0:3009`) says what the bus is doing.
The **control API** described here is HTTP + JSON, off by default, and is what
the Signal K plugin uses for everything else: which devices and fields exist,
what the mapping is, whether a proposed entry would work, and writing a field
when Signal K receives a PUT.

Turn it on with `api_listen` in `config.ini` or `--api` on the command line.

```ini
api_listen = 127.0.0.1:3010
# api_token = change-me      # required unless api_listen is loopback
```

Every request must carry `Authorization: Bearer <token>` when a token is
configured. Any address other than loopback refuses to start without one.

Responses are JSON. Errors are `{"error": "<message for a human>"}` with a
4xx or 5xx status; a `502` means the bus did not cooperate (timeout, device
not answering) and is worth retrying.

## Versioning

`GET /api/status` carries `apiVersion`. It is **1** for the shapes below and
is bumped when a response changes incompatibly. Adding a key is not
incompatible. A client should refuse a version it does not know.

## Identifiers

- A **device** is addressed by its **serial number**, the mapping file's key.
  A device that has not reported a serial cannot be mapped or addressed.
- A **field** is addressed by its channel-aware id as three hex digits with a
  `0x` prefix, `0x000`..`0x1FF`, the same encoding the TUI and
  `masterbus-set-field` use.
- A **menu** is `monitoring`, `configuration` or `service`.

## Endpoints

### `GET /api/status`

```json
{
  "apiVersion": 1,
  "version": "0.4.0",
  "transport": "can:can0",
  "stream": "0.0.0.0:3009",
  "api": "127.0.0.1:3010",
  "uptime": 3612,
  "devices": 13,
  "mapped": { "devices": 4, "fields": 25 },
  "streaming": 25,
  "clients": 1,
  "diagnostics": { "errors": 0, "warnings": 1 }
}
```

`streaming` is the number of mapped fields the daemon is actually subscribed
to (entries with an error diagnostic are not). `clients` counts stream
connections.

### `GET /api/devices`

Every device discovered so far, sorted by bus address, each with the fields
of every menu discovered on it (Monitoring at least).

```json
[
  {
    "id": "188EA2",
    "serial": "R516V1070",
    "article": "26024000",
    "name": "MSU Inverter",
    "firmware": "2.65",
    "instance": "inverter",
    "menus": ["monitoring"],
    "mapped": 3,
    "fields": [
      {
        "id": "0x006",
        "name": "Main battery",
        "unit": "V",
        "options": [],
        "writable": false,
        "menu": "monitoring",
        "group": "Battery",
        "path": "electrical.inverters.inverter.dc.voltage",
        "put": false,
        "value": 25.6
      }
    ]
  }
]
```

- `instance` is the Signal K instance the mapping records for the device, or
  the one that would be proposed for it.
- `options` are an enum's labels; empty otherwise.
- `path` and `put` come from the mapping; `path` is `null` for an unmapped field.
- `value` is the last value the stream saw, **in the device's own unit**
  (the mapping's SI conversion is not applied), or `null` if none has been
  seen. Only mapped fields are streamed, so an unmapped field has a value
  only after `GET .../value` below.

### `GET /api/devices/{serial}`

One device, same shape. With `?menu=configuration` (or `service`) that menu
is discovered first if it has not been, and its fields are merged in. A cold
discovery can take a few seconds. `404` for an unknown serial, `400` for an
unknown menu, `502` if the device would not enumerate.

### `GET /api/devices/{serial}/fields/{id}/value`

Reads the field now (from the engine's cache when fresh, else the bus) and
answers `{"value": <raw>}` in the device's own unit. The value is also
remembered for the listing above.

### `PUT /api/devices/{serial}/fields/{id}`

Writes a field. This is what a Signal K PUT turns into.

```json
{ "value": true }
{ "value": 293.15 }
{ "value": "float", "login": { "level": "installer", "code": 1234 } }
```

The value is interpreted **as the path publishes it** when the field is
mapped: an SI number is converted back to the device unit, a boolean is
negated by `invert`, an enum with a truth table takes `true`/`false` and
picks the matching label. An enum without a truth table takes a label
(case-insensitive) or an option index. An unmapped field takes the value in
the device's own unit.

A field that is read-only at the device's current access level answers
`403` with `"needs": "login"`. Supplying `login` makes the daemon log the
device in at that level and retry once; `level` is `installer`,
`distributor` or `mvservice`, and `code` is the vendor's numeric code. A
rejected code is also `403`.

Success:

```json
{ "applied": false, "published": true }
```

`applied` is the value observed on the device after the write, in its own
unit; `published` is what the stream will now carry for the path (`null`
for an unmapped field or a value the path cannot represent).

### `GET /api/mapping`

The mapping in force, in the `mapping.json` format (see
`crates/masterbus-tools/README.md`).

### `PUT /api/mapping`

Replaces the mapping wholesale. The body is a complete mapping document. It
is validated against the devices on the bus, written to `mapping.json`, put
in force immediately, and the answer carries the diagnostics:

```json
{
  "mapped": 26,
  "streaming": 25,
  "diagnostics": [
    {
      "severity": "error",
      "serial": "R516V1070",
      "device": "MSU Inverter",
      "field": "0x00A",
      "path": "electrical.inverters.inverter.ac.temperature",
      "message": "\"V\" cannot be converted to the K this leaf expects; skipped"
    }
  ]
}
```

`severity` is `error` (the entry is skipped), `warning` (it publishes, but
look at it) or `info`. `field` and `path` are absent for a device-level
diagnostic (a device not on the bus, a firmware mismatch). A mapping that
does not parse, or carries a `version` this build does not write, is
refused with `400` and nothing changes; a mapping that cannot be written to
disk is refused with `500`.

An entry may carry `"put": true` to say the plugin should accept Signal K
PUTs on its path. A field outside Monitoring (the writable settings live on
Configuration) is fine: the daemon discovers that menu when the mapping
names one of its fields.

### `GET /api/mapping/diagnostics`

The diagnostics of the mapping in force, same shape as above.

### `POST /api/mapping/suggest`

```json
{ "serial": "R516V1070", "field": "0x006" }
```

What the editor pre-fills for a field:

```json
{
  "path": "electrical.inverters.inverter.dc.voltage",
  "invert": false,
  "tier": "model",
  "truthDefault": null,
  "notifyDefault": {}
}
```

`tier` says where the proposal came from: `existing` (the field is already
mapped; this is its entry), `modelFirmware` or `model` (the bundled
per-model database), `name` (the per-class name heuristics), or `null`
(nothing known; `path` is then a prefix to type after, the node the device
already publishes into or `electrical.`). `truthDefault` is the conventional
truth table for an enum's labels when every label is unambiguous, else
`null`; `notifyDefault` the labels that conventionally deserve a
notification.

### `POST /api/mapping/validate`

```json
{ "serial": "R516V1070", "field": "0x005", "entry": { "path": "electrical.batteries.house.temperature" } }
```

What saving the entry would do, worked out the way the daemon will. Always
`200`:

```json
{ "ok": true, "unit": "K", "conversion": "×1 +273.15", "boolean": false,
  "truth": {}, "notify": {}, "invert": false, "warnings": [] }
```

or

```json
{ "ok": false, "refusal": { "kind": "units", "message": "\"A\" cannot be converted to the K this leaf expects", "labels": [] } }
```

`kind` is `units`, `truth` (a boolean leaf needs a truth table for the
listed `labels`) or `empty`. `warnings` are things the daemon would say at
activation but still publish: a unit it cannot convert, three or more labels
onto a boolean leaf, a `put` on a field that is read-only right now.

### `POST /api/mapping/apply-article`

```json
{ "serial": "R516V1070" }
```

Copies this device's mapping onto every other device with the same article
number, substituting each target's Signal K instance into the paths and
skipping fields a target does not have. Persists and activates like
`PUT /api/mapping`.

```json
{ "targets": 3, "copied": 9, "skipped": 3, "diagnostics": [] }
```

## The delta stream

Unchanged: connect to the stream port and read newline-delimited Signal K
deltas with `$source: "masterbus"`. On connect a client receives the static
per-device metadata (`name`, `manufacturer`) and, with the next value of
each notifying field, the current notification states. Unit `meta` is sent
once per path before its first value. A plugin republishing the stream
through `handleMessage` should drop `$source` and `timestamp` and keep
`values` and `meta`.

## Running under a supervisor

- `--config-dir DIR` (or `MASTERBUS_CONFIG_DIR`) puts `config.ini`,
  `mapping.json` and the schema cache under one directory.
- `--api-token-file PATH` reads the token from a file instead of the
  command line or config.
- Once both listeners are bound the daemon prints one line on stdout:
  `READY {"stream":"127.0.0.1:3009","api":"127.0.0.1:3010","version":"0.4.0"}`.
  Discovery continues after that; `/api/status` shows devices arriving.
