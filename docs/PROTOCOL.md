# Mastervolt MasterBus CAN Protocol

Wire-level reference for the Mastervolt MasterBus protocol as observed on a live
bus (`candump`, `strace`) and confirmed against the original `libmasterbus.so`
(32-bit ARM). This documents the transport, frame layout, query/response
exchanges, and value encodings used to enumerate devices and read monitoring
data.

> Companion document: see `FINDINGS.md` for how these facts were reverse
> engineered, the original library's runtime behaviour, and reimplementation
> notes.

---

## 1. Transport

MasterBus runs on a standard CAN bus using **29-bit extended frames (EFF)**.

### 1.1 29-bit CAN ID layout

```
bits 28:24  can_class   (5 bits — frame role)
bits 23:0   device_addr (24 bits — unique per device)

can_id = (can_class << 24) | (device_addr & 0x00FF_FFFF)
```

Frames must be transmitted/received as **extended** frames, not standard.

### 1.2 Linux SocketCAN `can_frame` (16 bytes)

```
[0:4]   can_id   u32 little-endian   (bit 31 = CAN_EFF_FLAG,
                                      bit 30 = RTR, bit 29 = ERR)
[4]     can_dlc  data length (0..8)
[5:7]   padding  (3 bytes)
[8:16]  data[8]  payload
```

Strip the top 3 flag bits (`& 0x1FFF_FFFF`) before extracting `can_class` /
`device_addr`.

---

## 2. CAN classes

Class is bits 28:24 of the CAN ID. Observed classes:

| Class | Name              | Dir | Meaning                                                   |
|-------|-------------------|-----|-----------------------------------------------------------|
| 0x04  | DEVICE_BROADCAST  | ←   | Periodic 8-byte device self-announcement                  |
| 0x05  | BUS_POLL          | ←   | Bus-master heartbeat (0-byte frame from node `0x500001`)  |
| 0x06  | PROPERTY_INFO     | ←   | Property / group-count response from a device             |
| 0x07  | PROPERTY_REQ      | →   | Property / group-count request to a device                |
| 0x08  | MONITORING_DATA   | ←   | Monitoring value pushed from a device (also Btm1 metadata response) |
| 0x09  | SCHEMA_DATA       | ←   | Schema response from a device (full 8-byte form)          |
| 0x10  | WRITE_ACK / NA    | ←   | Write acknowledgement; Btm1 metadata "no value" response  |
| 0x11  | SCHEMA_DATA_NA    | ←   | Compact / NA schema response (4-byte form, see §5)        |
| 0x18  | MONITORING_REQ    | →   | Monitoring request to a device (also Btm1 metadata request) |
| 0x19  | SCHEMA_REQ        | →   | Schema request to a device                                |
| 0x1A  | SCHEMA_REQ_ALT_A  | →   | Alternate schema request (variant — semantics unconfirmed)|
| 0x1B  | SCHEMA_REQ_ALT_B  | →   | Alternate schema request (variant — semantics unconfirmed)|
| 0x1C  | BTM3_META_REQ     | →   | Btm3 per-field metadata request (see §6)                  |

`→` = controller→device (request), `←` = device→controller (response/push).

Classes `0x1A` / `0x1B` / `0x1C` are observed in captures of MasterAdjust
querying group-level metadata on devices whose `0x19` schema returns empty
or is silent (see §4.3 caveat and FINDINGS). Their full opcode space hasn't
been reversed; treat them as TBD and refer to live capture before relying
on a specific interpretation.

There is **no correlation token** in responses. Requests and responses are
matched by `(device_addr, class, payload-key)` — see §8.

---

## 3. Device discovery (class 0x04)

Devices announce themselves *periodically* via 8-byte broadcasts; discovery is
**passive** — listen, don't poll. Broadcasts arrive frequently (< ~2 s apart),
so a short collection window captures the whole bus.

### Broadcast payload (8 bytes)

```
[0]   type_code         device family
[1:3] addr_lo, addr_hi  low 16 bits of device_addr (mirror of CAN ID)
[3]   instance          sub-device instance (0..3)
[4]   event_counter     ticks (+1 or +4) on state changes — see §4.5
[5]   (status / flags — bits not fully decoded)
[6:8] (unidentified)
```

The 24-bit `device_addr` comes from the CAN ID, not the payload.

> Byte `[4]` is **not** a login-state indicator. It increments on a wide
> range of device-state changes (login, logout, value writes, and others
> we haven't characterised) — so a transition is a useful "device woke up
> and refreshed" signal, but the value alone doesn't reveal *what*
> changed. To read the access level, query opcode `0x08 0x19` (§4.5). The
> earlier theory that bytes `[4:6]` encoded firmware version did not hold
> up across devices.

---

## 4. Property queries (class 0x07 request → 0x06 response)

All requests are class `0x07` to `device_addr`; responses are class `0x06`.

### 4.1 Firmware version — `[0x82, n]`

Two queries, `n = 0x00` and `n = 0x01`. Each response is `[0x82, n, b2, b3]`.
The displayed version is computed by summing across both responses:

```
major = resp(n=0)[3] + resp(n=1)[3]
minor = resp(n=0)[2] + resp(n=1)[2]
version = "{major}.{minor}"      e.g. "1.37", "7.88", "0.122"
```

(Empirically verified to reproduce the original output for all devices.)

The resulting `major.minor` is the device's real firmware version, and it
identifies the exact vendor firmware image: MasterAdjust's aggregated
`SoftwareVersion` for the same device is `(major << 8) | minor`, and that value
matches the vendor's published filenames — `0x020E` = 2.14 =
`Easy_5_V2.14.hex`, `0x0204` = 2.4 = `Digtal_Input_Switch_V2.04.hex`. Useful
for keying an offline string-table catalog (§4.4).

### 4.2 Property string id — `[0x09, n]`

Returns the string-table id for a named device property.

| n | Property      |
|---|---------------|
| 1 | article       |
| 2 | serial number |
| 3 | device name   |
| 4 | revision      |

Response: `[0x09, n, sid_lo, sid_hi]` → `str_id = u16LE(sid_lo, sid_hi)`.

- **`str_id` may legitimately be `0`** — id 0 is a real entry in the string
  table (it is the serial number on several devices). Distinguish "no response"
  (timeout) from "id 0"; do **not** treat 0 as absent here.
- Query **n=4 (revision) is often unanswered**; in that case the revision is
  derived as the **5th character of the serial** (index 4). When n=4 *does*
  answer, it is a full code string (e.g. `"DD0012"`, `"AA0101"`). See FINDINGS.

### 4.3 Property-by-selector — `[0x08, SEL]`

A wide family of small counters/sizes is keyed by a single selector byte:
request `[0x08, SEL]` on class `0x07`, response `[0x08, SEL, lo, hi]` on
class `0x06` with `u16LE(lo, hi)`. Selectors that matter:

| SEL    | Meaning                                                                  |
|--------|--------------------------------------------------------------------------|
| `0x01` | **Total field-index range** — `N` such that the device's addressable field indices are `0..N`. The basis for the flat probe in §4.6. |
| `0x02` | Number of **Monitoring** groups (selector for the schema offset, §5).    |
| `0x03` | Number of **Configuration** groups. **Unreliable** on some devices — see caveat below. |
| `0x04` | Number of **Service** groups.                                            |
| `0x3F` | **Total tab count** (`mon + alarm + history + config`), *not* a per-tab count. |

> **`[0x08, 0x03]` is unreliable** on Magic-class devices (and likely
> others). The Mac/Magic family — e.g. the **INT Nav Chg** at `0x3A3B4B`
> in the reference bus — returns `0` for `[0x08, 0x03]` and `[0x08, 0x04]`
> despite exposing ~25 configurable fields. The vendor's MasterAdjust
> application papers over this with hard-coded layout knowledge built
> into per-device-family Delphi classes (`TDevice_MacMagic`,
> `TDevice_HFcombi`, `TDevice_Mass2And3`, …) recovered from
> `MasterAdjust.exe`; the section names ("Main", "Stabilize voltage",
> "Dimmer", "Charger", …) and field grouping come from those classes,
> not from the wire. See FINDINGS §3e for the capture that proved this.
>
> For a transport-only client, **don't trust `0x03` as ground truth**;
> use the flat probe (§4.6) and accept losing the per-section grouping.

Other selectors observed in MasterAdjust traces (`0x05`, `0x06`, `0x07`,
`0x08`, `0x12`, `0x13`, `0x1B`–`0x1F`, `0x23`, `0x3E`) return values whose
exact meaning hasn't been pinned down across devices. On the INT Nav Chg
`[0x08, 0x06]` returns `5`, which happens to match the number of
Configuration sections in the MasterAdjust UI, but the same selector on
the CombiMaster returns `14`, so it isn't a generic "section count" —
treat it as TBD.

### 4.4 String-table chunk — `[0x30, id_lo, id_hi, seq]`

Fetches a string by id, 4 characters per chunk, starting at `seq = 0`.

Response: `[0x30, id_lo, id_hi, seq, c0, c1, c2, c3]` (chars truncated on the
last chunk).

Termination: stop when a chunk contains a `NUL` (0x00) byte **or** is shorter
than 8 bytes (fewer than 4 chars). Otherwise request `seq + 1`.

**The id space is split.** Low ids are device-specific and held in EEPROM —
serial number, user-assigned device name, article number. Ids above that are a
flat index into a **static table baked into the firmware image**, identical for
every unit running the same firmware. The boundary is per-model (id 6 on the
Repeater, id 18 on the Digital Input Switch); there is no marker on the wire
that tells you where it falls.

**This is the dominant cost of discovery.** Four characters per round trip
means a device reporting 324 strings (an MLI Ultra) costs on the order of a
thousand exchanges. Because the flash half of the table is static per firmware
build, it can be recovered offline from the vendor's public firmware bundle and
shipped alongside the library — see FINDINGS §4.4 for the extraction method,
which is validated id-for-id against a live 14-device capture.

**`StringVersion` is the cache key.** The `GeneralCount` block reports
`StringVersion` next to `ArticleNumber` (§4.3). A pre-built table is valid
exactly when *both* match the device; on any mismatch, fall back to fetching
over the wire. Do not key on article number alone — firmware revisions
renumber the table (`Factory reset` at flash index 27 in `Repeater_V1.00` is
`Factory settings` on the unit captured on the reference bus).

Even with a matching key, **spot-check before trusting a bundled table**: fetch
two or three ids spread across the range and compare. That costs three round
trips instead of a thousand and removes any need to be certain the offline
extraction was correct for a model you have never seen.

The firmware also stores **every UI language**, not just the configured one, so
a bundled table can offer languages the device itself will not serve. Language
ordering in flash is *not* the on-wire language enum — see FINDINGS §4.5.

Three rules a bundled table must respect (all learned the hard way — see
FINDINGS §4.6, where the approach is validated at 420/420 against a live bus):

- **Carry an explicit id range.** Reading past the end of a device's table
  yields plausible-looking but wrong strings rather than an error.
- **Never cache an entry the firmware leaves empty.** Empty means "no static
  text": the device supplies it from EEPROM (user-assigned switch and device
  names) or generates it at runtime from a template (`Event 3 command`,
  `P: 2`). Always fetch those live.
- **Never cache across a version mismatch.** It corrupts wholesale rather than
  degrading — a Repeater running 0.61 read against the 1.0 table gets roughly
  half its strings wrong.

### 4.5 Access-level login — opcode `0x08 0x19`

Property opcode `0x08 0x19` is the **access level** register. It has three
forms — read, write (login), and logout-write — all on class `0x07` to a
single `device_addr` (per-device, not broadcast):

| Form    | Dlc | Payload                                    | Meaning |
|---------|-----|--------------------------------------------|---------|
| read    | 2   | `08 19`                                    | request: "what level am I at?" |
| login   | 8   | `08 19 <level> 00 <f32 code, LE>`          | request: "set level (validated by code)" |
| logout  | 4   | `08 19 00 00`                              | request: "back to End User" |

The response is class `0x06`, payload `08 19 <level> 00` (4 bytes) in all
three cases: it echoes the **current** level after the request was applied.
Latency ~3–6 ms. A wrong code presumably gets a `level == 0` echo or no
response; not observed (the MasterAdjust GUI rejects bad codes client-side).

**Access levels and codes** (recovered live + by decompiling MasterAdjust;
see FINDINGS §3c/3d):

| Level byte | Name (MasterAdjust label) |
|------------|---------------------------|
| `0x00`     | **End User**              |
| `0x01`     | **Installer**             |
| `0x02`     | **Distributor**           |
| `0x03`     | **MV Service**            |

The f32 access codes for levels 1..3 are vendor-defined per device family
and are **not** carried in this crate. The encoder takes whichever code the
caller supplies and packs it onto the wire as-is; if the device doesn't
accept the code it silently keeps the prior level (compare the new level
to the previous one to detect a rejected attempt).

The level byte in the request must match the code (each level has a
distinct code; sending the wrong code for a level leaves the device at
its previous level). The code is the integer's IEEE-754 float
representation, not ASCII and not a packed integer — the same "every
value is a 4-byte float" rule as §7.3.

**Effect of a successful login:**

- Subsequent reads of `08 19` return the new level byte.
- Field WRITEABLE responses (meta opcode `0x0B`, §6) change for many
  fields — the multi-byte value differs per level. The bitmask layout is
  not yet fully decoded; the practical rule is **re-query `0x0B` for every
  field after any level change** rather than caching pre-login attributes.
- The next DEVICE_BROADCAST (class `0x04`) carries an updated `data[4]`,
  but that byte is a per-device **event counter** (increments by 1–4 on
  state changes, also ticks for non-login events) — **not** a level
  indicator. To know the current level, query `08 19`.
- **Schema does not grow.** Same field/group indices remain reachable.
  Login changes attribute responses, not the index space.

**Scope and persistence.** Login is per-device. Persistence across device
power cycles has not been characterised. MasterAdjust polls `08 19` on
every enrolled device periodically (~once per minute in the reference
capture) so it can resync its UI to actual device state.

### 4.6 Flat field-index probe (group-count fallback)

Because `[0x08, 0x03]` lies on some devices (§4.3), the only universally
reliable way to enumerate a device's settings is to probe the **full
field-index space** directly via Btm1 metadata (§6), bypassing groups
entirely.

Algorithm:

1. Read `[0x08, 0x01]` to get `N` — the device's total field-index count.
2. For each `field_idx ∈ 0..N`, send the pipelined metadata batch
   (`NAME / VIZ / MAX / UNIT / WRITEABLE`) to the Btm1 metadata address (§6).
3. Drop indices that produce no response in any opcode of the batch —
   those are holes in the index space and not addressable fields.
4. What remains is every reachable field, **without group structure**.

This is what `Device::all_fields()` does in the crate. Tradeoffs vs.
the group-based discovery in §5:

- **Pro:** Recovers fields on devices whose `0x08, 0x03` count is `0`
  (e.g. Magic-class). On the INT Nav Chg this returns the ~25 fields
  MasterAdjust shows; the `0x19` schema path returns nothing.
- **Pro:** Works without per-device-family knowledge — no hard-coded
  `TDevice_*` analogue is needed.
- **Con:** No section labels. MasterAdjust's "Serial interface / Main /
  Stabilize / Dimmer / Charger" headings come from a hard-coded layout
  inside `MasterAdjust.exe` (one Delphi class per device family); the
  bus itself never exposes them. Recovering those headings means RE'ing
  the Delphi classes and porting them into the client.
- **Con:** Linear cost in `N` metadata batches (~150 ms each at our
  per-attempt timeout). The Nav Chg's `N = 64` takes a few seconds on a
  cold cache; subsequent visits are free.

A login (§4.5) invalidates the flat-probe cache the same way it
invalidates the group cache: writability bits change per level and stale
attributes would lie to the caller.

---

## 5. Schema queries (class 0x19 request → 0x09 / 0x11 response)

Per-group structure. Requests are class `0x19` to `device_addr`. Responses
arrive on either class `0x09` (the rich form, 8 bytes) or class `0x11`
(compact / NA, 4 bytes). Result fields live at `bytes[4:..]` of the
class-`0x09` response.

| Purpose         | Request payload              | Class-0x09 response value              |
|-----------------|------------------------------|----------------------------------------|
| Group name id   | `[0x28, group_id, 0x00]`     | `u16LE` at bytes[4:6] (string id)      |
| Field count     | `[0x07, group_id, 0x00]`     | `f32LE` at bytes[4:8] (count)          |
| Field id        | `[0x03, group_id, 0x00, idx]`| `u16LE` at bytes[4:6] (field id)       |

The field-id response echoes `idx` in `bytes[3]` (used for matching, §8).

**Class `0x11` compact / NA response.** On the CombiMaster reference bus,
schema queries for Configuration-tab group ids consistently came back on
class `0x11` with a 4-byte payload `[opcode, group_id, sub, value]`,
e.g. `Up 11 188EA2 [4] 07 07 00 03`. The `value` byte was inconsistent
across retries of the same query (e.g. `03`, `01`, `01` for three
back-to-back probes of gid 7's field count), which is more consistent
with a "not-available / try-again" status than a real value. The crate
currently ignores `0x11` and treats the request as silent; treat the
byte at `[3]` as TBD until further captures clarify whether it carries
useful data.

**Group-as-field requests (class `0x1C`).** MasterAdjust separately
queries metadata-style opcodes (`0x02 VIZ`, `0x07 MAX`, `0x09 FACTORY`,
`0x0B WRITEABLE`, `0x0C GRAY`, `0x28 NAME`) on **group ids** via class
`0x1C` with payload `[opcode, gid, 0x00]`. Responses come back on class
`0x11` with the 4-byte compact form. This appears to be how MasterAdjust
treats a Configuration group as a single field-like row when its layout
table tells it to (see §4.3 caveat). Semantics still TBD; not used by
the crate.

---

## 6. Per-field metadata

There are **two parallel metadata channels** named after the vendor's Delphi
units that handle them (`Btm1.pas` and `Btm3.pas` in MasterAdjust). Same
per-field opcode set on both; only the CAN class and the addressing differ.
Both expose the same conceptual data — name / viz / min / max / step /
factory default / writeable / unit / option strings — but on **separate
field-index namespaces**: index `0x17` on Btm1 is a different field than
index `0x17` on Btm3, with different name and value (see FINDINGS §3f).

**Btm1 metadata** (the legacy / standard channel):

```
btm1_meta_addr = (device_addr | 0x800000) & 0x00FF_FFFF
```

Request: class `0x18` to `btm1_meta_addr`, payload `[opcode, field_lo, field_hi]`.
Response: class `0x08` from `btm1_meta_addr` (the high bit `0x800000` is
what distinguishes a Btm1 metadata response from a real monitoring push).

**Btm3 metadata** (newer revision; required by Magic-class devices like the
INT Nav Chg, also supported by HFcombi devices in addition to Btm1):

Request: class `0x1C` to the device's **real** address, payload
`[opcode, field_lo, field_hi]`. Response: class `0x0C` from the real
address (the channel uses no address-flag bit; class disambiguates).

Btm3 values arrive on a separate path: passive class `0x0B` pushes
addressed to `addr | 0x800000` with a headerless `[fid_lo, fid_hi, b0..b3]`
payload; writes go out on class `0x1B` to the same address with the same
payload format.

Each field attribute has its own opcode. The full attribute set (the vendor's
`pit*` "property-item-type" enum, recovered by decompiling MasterAdjust.exe —
`FUN_00490720` maps opcode → attribute):

| opcode | Attribute (`pit*`)      | Response value                         | Notes |
|--------|-------------------------|----------------------------------------|-------|
| 0x02   | Visualization_Type      | byte[4] (see §7.1)                      | field's display/edit widget |
| 0x03   | GroupItemList           | —                                      | group membership |
| 0x06   | Minimum                 | `f32LE` at bytes[4:8]                   | edit range lower bound |
| 0x07   | Maximum / option count  | `f32LE` at bytes[4:8]                   | max value, or list option count |
| 0x08   | Step                    | `f32LE` at bytes[4:8]                   | edit increment |
| 0x09   | Factory_Default         | `f32LE` at bytes[4:8]                   | default/reset value |
| 0x0B   | **Writeable**           | flag at byte[4]                        | **field is writable iff set** |
| 0x0C   | Gray_Visualization      | byte[4]                                | greyed/read-only display hint |
| 0x0D   | Eventable               | —                                      | eventable list metadata |
| 0x26   | List_Text (option str)  | `u16LE` at bytes[4:6]; req `[0x26, field, 0x00, opt_idx]` | per-option string id |
| 0x28   | Text_Value (name)       | `u16LE` at bytes[4:6]                   | field name string id |
| 0x29   | Text_Value_Long         | `u16LE` at bytes[4:6]                   | longer text string id |
| 0x2A   | Text_Value_Very_Long    | `u16LE` at bytes[4:6]                   | longest text string id |
| 0x2C   | Unit_String             | `u16LE` at bytes[4:6] (**often absent**)| unit string id |

- **Writability is explicit**: query `0x0B` per field; do not infer it from viz
  type (e.g. a settable "AC IN limit" and a read-only "Input voltage" are both
  numeric). Confirm the flag's byte width on a live bus.
- **A metadata string id of `0` means "no string"** — the opposite of
  property string ids (§4.2). Unitless fields often return no `0x2C`
  response at all.
- Option strings (`0x26`) are only present for list-type fields and only up to
  `option count` (`0x07`).
- Opcodes `0x02/0x07/0x26/0x28/0x2C` are confirmed on the wire from live capture;
  the others (`0x03/0x06/0x08/0x09/0x0B/0x0C/0x0D/0x29/0x2A`) are from the
  MasterAdjust decompile and should be confirmed live when used.

String ids returned by metadata queries are fetched with the same chunk
mechanism as §4.4 (to the real `device_addr`, not the Btm1 metadata address).

---

## 7. Monitoring data (class 0x18 request → 0x08 response)

### 7.1 Read a field value

Request: class `0x18` to `device_addr`, payload `[field_index, tab_index]`.
Response: class `0x08`, payload `[field_index, tab_index, b0, b1, b2, b3]` — a
4-byte value whose interpretation depends on the field's visualization type.

Tab indices: `0 = Monitoring`, `1 = Alarm`, `2 = History`, `3 = Configuration`.

### 7.2 Write a value

Writes use the **same class `0x18`** as a read, but carry the field's full
**4-byte value** after `[field_index, tab_index]`. Every settable field — numeric,
boolean, or list — is written as the same 4-byte representation it is read back as:

```
[field_index, tab_index, b0, b1, b2, b3]
```

| Field type | 4-byte value                                     |
|------------|--------------------------------------------------|
| Float      | IEEE-754 single, little-endian                   |
| Boolean    | float `1.0` (`00 00 80 3f`) / `0.0` (`00 00 00 00`) |
| List/enum  | the selected index as a float (`1.0` = option 1) |

A **1-byte boolean write is ignored** by at least the CombiMaster — booleans must
be the full 4-byte float. The device confirms by pushing the new value (class
`0x08`); the immediate echo is unreliable, so confirm by re-reading.

### 7.3 Relay-control "commit" token

Some controls do **not** act on the value write alone. Toggling the CombiMaster's
**inverter** or **charger** emits the value write **plus** a second write to an
**adjacent hidden command register** at `field_index + 1`, carrying a fixed token:

```
inverter on:   [0x13, 0x00, 00 00 80 3f]   then  [0x14, 0x00, 14 9f 3c 02]
charger  off:  [0x15, 0x00, 00 00 00 00]   then  [0x16, 0x00, 14 9f 3c 02]
```

The token `14 9f 3c 02` is **constant** — identical for inverter and charger, for
on and off, and across sessions (not a counter, timestamp, or checksum). The
command register (`0x14`, `0x16`) is **write-only**: absent from the schema, it
returns no value when read. Without the commit write the value write is accepted
but the relay does not change.

A safe rule for emitting it: send the token after a boolean write **only when
`field_index + 1` is not a real schema field**, so devices that don't use this
pattern are never disturbed. Float and list writes (e.g. an AC-input limit or a
drop-down) need no commit write.

---

## 8. Request/response matching keys

Because responses carry no token, each in-flight request is keyed and matched
against incoming frames:

| Exchange                | Key                                            |
|-------------------------|------------------------------------------------|
| Monitoring value (0x08) | `(device_addr, tab_index, field_index)`        |
| Property (0x06)         | `p:{addr}:{data[0]}:{data[1]}`                 |
| String chunk (0x06,0x30)| `str:{addr}:{str_id}:{seq}` (seq from data[3]) |
| Schema (0x09)           | `schema:{addr}:{op}:{group}` (+`:{idx}` for 0x03 field-id, idx in data[3]) |
| Btm1 metadata (0x08, hi bit) | `btm1_meta:{real_addr}:{op}:{field}` (+`:{opt}` for 0x26, opt in data[3]) |
| Btm3 metadata (0x0C)    | `btm3_meta:{addr}:{op}:{field}` (+`:{opt}` for 0x26, opt in data[3]) |

Regular monitoring pushes (class 0x08, no `0x800000` bit) are routed to the
value table; Btm1 metadata responses (class 0x08 with the `0x800000` address
bit) are routed to the metadata waiter.

---

## 9. Value encodings (4-byte payload)

Decoded according to the field's visualization type (§6, opcode 0x02). Observed
wire codes for `0x02`:

| Wire code | Visualization |
|-----------|---------------|
| 0x01      | Float         |
| 0x03      | DropDown      |
| 0x04      | Eventable     |
| 0x05      | CheckBox      |
| 0x06      | Text          |
| 0x07      | Time          |
| 0x08      | Date          |
| 0x09      | DeviceList (event target — a device reference) |
| 0x0A      | EventCommand (which of the target's eventable outputs) |

The full set was recovered by matching MasterAdjust's captured
`VisualizationType` field against known field types — the values agree with the
wire codes on every type observed both ways (`0x01`/`0x03`/`0x06`/`0x07`/`0x08`),
which fixes `0x09`/`0x0A`. `0x02` (a greyed float) and `0x0B` (a switch variant)
are also seen but not yet needed.

### Float / GrayVisualization
IEEE-754 single, little-endian. The quiet-NaN `0x7FC00000` means
"unknown / not available" (render as null).

### Boolean (CheckBox / ToggleButton / PushButton)
The value is a 4-byte float (`1.0` = on, `0.0` = off), so decode as **any non-zero
byte in the 4-byte value**. An "on" arriving as the float `1.0` (`00 00 80 3f`)
has `byte[0] == 0`, so testing only `byte[0]` would misread it as off.

### Time  → `DD:HH:MM:SS`
The 4 bytes are an IEEE-754 single holding **total seconds** `t`:

```
days  = t / 86400
hour  = (t % 86400) / 3600
min   = (t % 3600)  / 60
sec    = t % 60
```

NaN / ±inf ⇒ null. Example: `54496.0 s` → `00:15:08:16`.

### Date  → `DD/MM/YYYY`
The 4 bytes are an IEEE-754 single whose **integer value `T` packs the date**:

```
T = year*416 + month*32 + day          (416 = 13 * 32)

day   = T % 32
month = (T / 32) % 13
year  = T / 416
```

NaN / ±inf ⇒ null. Example: `843000.0` → `2026*416 + 5*32 + 24` → `24/05/2026`.

> The library stores dates internally in a different packed form
> `(year<<13)|(day_of_year<<4)|frac` with a 733-byte day-of-year→month/day
> table, but the **wire value** is exactly `year*416 + month*32 + day`. This was
> verified bit-for-bit against the library across every date from 2000–2100,
> including leap years. See FINDINGS §3.

### Radio / DropDown
The selected **index as a 4-byte float** (e.g. `1.0` = option 1), not a low byte:
the Solar "Override" drop-down read/wrote `00 00 80 3f` for option 1. Decode as
`round(f32)`. The human-readable label comes from the option strings (§6 opcode
0x26).

### Eventable / DeviceList / Event command
The selected **index as a 4-byte float**, same as Radio/DropDown (`round(f32)`).
Decoding `byte[0]` alone is wrong — a small index like `2.0` has a zero low
byte.

### Text
UTF-8 bytes of the payload (lossy decode).

---

## 9a. Events

A device can act on other devices when a condition occurs. Each event is **four
consecutive config fields**:

| field | viz | meaning |
|-------|-----|---------|
| `Event N source`  | 0x03 DropDown   | the trigger condition on this device (device-specific list, e.g. `Stop charge`, `Battery low`) |
| `Event N target`  | 0x09 DeviceList | **which device to act on** |
| `Event N command` | 0x0A            | **which of the target's eventable outputs** |
| `Event N data`    | 0x03 DropDown   | the action: `Off` / `On` / `Copy` / `Copy invert` / `Toggle` |

**The target is an index into the bus device list sorted by device address
ascending** — the exact order `masterbus_devices()` (and this crate's
`device_ids()`, which sorts by `addr`) returns. So `target = 2` is the third
device in that sorted list. Confirmed against the reference bus: a battery's
"Stop charge → Solar → Off" event stores target `3`, and Solar is at sorted
index 3. See FINDINGS §8 for the derivation.

The `command` field selects **which of the target's eventable outputs** to
drive: its value is an index into the target device's eventable fields, in field
index order. A field is an eventable output when per-field metadata **op `0x0D`**
answers with `byte[4] == 1`. On the reference bus the Main Battery's outputs are
`Close relay` (**Monitoring** field 117) and `Open relay` (119); Solar's single
output is `Activate` (field 2). To show a command's label, resolve `command = K`
to the *target* device's `K`-th eventable field name — the target being the
sibling `Event N target`.

> **Eventable outputs are always Monitoring-tab fields** (100% of them across
> the 14-device reference bus; 0% on other tabs). Field indices are per-tab, so
> Monitoring field 117 (`Close relay`) is unrelated to Configuration field 117
> (`Cell 5`).
>
> **A device answers `0x0D` only for its eventable fields** — non-eventable
> fields don't reply. So querying it blocking on every field pays a full timeout
> on the silent majority. Query it *best-effort* (the reply, when it comes,
> arrives with the rest of the field's metadata batch) and only on Monitoring
> fields. MasterAdjust sidesteps this entirely by reading the *count* of
> eventables from the device-level selector `[0x08, 0x0D]` and a device-level
> eventable-list collection (`MB_Device_General_Count`); the wire query for the
> list items themselves isn't reversed yet — the per-field best-effort probe is
> what this crate uses.

---

## 10. Typical enumeration sequence (per device)

1. Learn the device exists from a class-0x04 broadcast.
2. Firmware: `[0x82,0]`, `[0x82,1]` (§4.1).
3. Property string ids: `[0x09,1..4]` → fetch each string (§4.2, §4.4);
   derive revision from serial[4] if n=4 is unanswered.
4. Monitoring group count: `[0x08,0x02]` (§4.3).
5. For each group: name id, field count, field ids (§5); fetch the group name.
6. For each field: metadata name/viz/max/unit (§6); fetch name & unit strings;
   fetch option strings for list types.
7. (Optional) pre-poll each field's value (§7.1).

For Configuration / Service (or any time `[0x08, 0x03]` returns `0`
despite the device clearly being configurable), skip steps 4–6 above for
that menu and fall back to the **flat field-index probe** (§4.6):

4'. Total field count: `[0x08, 0x01]` → `N`.
5'. For each `field_idx ∈ 0..N`: pipelined metadata batch (§6); drop
    indices with no response in any opcode.
6'. Render the survivors as a flat list — no group structure is available
    from the bus for these devices.
