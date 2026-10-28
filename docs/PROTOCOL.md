# Mastervolt MasterBus CAN Protocol

Wire-level reference for the Mastervolt MasterBus protocol as observed on a live
bus (`candump`, `strace`) and confirmed against the original `libmasterbus.so`
(32-bit ARM). This documents the transport, frame layout, query/response
exchanges, and value encodings used to enumerate devices and read monitoring
data.

> References to "FINDINGS" (here and in source comments) point to the
> maintainer's research notes on how these facts were reverse engineered, the
> original library's runtime behaviour, and reimplementation notes. That
> document is not published in this repository; every fact the code relies on
> is stated in this file.

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

`device_addr` is the device's **BusID** in compact form: bits 22:18 are its
product class and bits 17:0 a number derived from its serial; bit 23 always
clear (§3a). The firmware itself splits the id as a 6-bit frame type
(bits 28:23) and a 23-bit address, so `can_class` here is the frame type
shifted right by one; a frame to or from `addr | 0x800000` (§2.1) is the
odd frame type.

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
| 0x06  | PROPERTY_INFO     | ←   | Property / count response from a device (§4)             |
| 0x07  | PROPERTY_REQ      | →   | Property / count request to a device (§4)                 |
| 0x02, 0x03, 0x08–0x0D, 0x10–0x15, 0x18–0x1D | | ↔ | The four tabs' values, properties and groups (§2.1) |

`→` = controller→device (request), `←` = device→controller (response/push).

There is **no correlation token** in responses. Requests and responses are
matched by `(device_addr, class, payload-key)` — see §8.

### 2.1 The four tabs

Every device has the same four tabs, as MasterAdjust shows them:
**Monitoring**, **Alarm**, **History** and **Configuration**. Each tab has
its own groups (§5), its own per-field properties (§6) and its own values
(§7), and numbers its fields from 0: Monitoring field `0x17` and
Configuration field `0x17` are two different fields. Each of those three
kinds of message has a request, an answer and a "not available" answer,
each on a CAN class of its own, sent to or from either the device's real
address or the address with bit 23 set (`addr | 0x800000`, "high" below).

| Tab           | Values: req → answer / NA      | Properties: req → answer / NA | Groups: req → answer / NA |
|---------------|--------------------------------|-------------------------------|---------------------------|
| Monitoring    | `18` → `08` / `10` (real)      | `18` → `08` / `10` (high)     | `19` → `09` / `11` (real) |
| Alarm         | `03` → `02` (real) / `02` (high) | `19` → `09` / `11` (high)   | `1A` → `0A` / `12` (real) |
| History       | `1D` → `0D` / `15` (high)      | `1A` → `0A` / `12` (high)     | `1B` → `0B` / `13` (real) |
| Configuration | `1B` → `0B` / `13` (high)      | `1C` → `0C` / `14` (real)     | `1C` → `0C` / `14` (high) |

No two answers share a class and address half, so an answer names its tab
and kind. The table is MasterAdjust's (`mvMasterbusCanPacketTypes.pas`;
functions `0048f678`–`0048fd98`), where a message type is
`class << 1 | bit 23`. A MasterAdjust trace of a ChargeMaster Plus (#6) uses
every cell of it.

Each tab's number of groups and of values is a selector (§4.3):

| Tab           | Groups        | Values        |
|---------------|---------------|---------------|
| Monitoring    | `[0x08, 0x02]`| `[0x08, 0x01]`|
| Alarm         | `[0x08, 0x08]`| `[0x08, 0x07]`|
| History       | `[0x08, 0x04]`| `[0x08, 0x03]`|
| Configuration | `[0x08, 0x06]`| `[0x08, 0x05]`|

MasterAdjust creates all four tabs for every device, with no check of the
device's family, and reads each from its own groups. Earlier versions of
the crate and of this document knew only the Monitoring tab's classes and
the Configuration tab's properties and values (calling them the "Btm1" and
"Btm3" channels, after Delphi units that turned out to be the serial BTM
battery-monitor drivers). They counted Configuration groups with
`[0x08, 0x03]`, which is the number of History values, and read them as
Monitoring groups past Monitoring's own count; on a ChargeMaster Plus that
gave groups such as "DSP Debug" and a group counting 54,650 fields, which
MasterAdjust never shows. They also had a "Service" menu, counted with
`[0x08, 0x04]`, which is the number of History groups; no device has a
Service tab.

---

## 3. Device discovery (class 0x04)

Devices announce themselves *periodically* via 8-byte broadcasts; discovery is
**passive** — listen, don't poll. Broadcasts arrive frequently (< ~2 s apart),
so a short collection window captures most of the bus — but not reliably all
of it. On a 13-device bus the busy half announces within the first second and
the quiet half (interfaces, the display, an idle charger) at or just after
2 s, drifting either side of the mark from run to run (#22). A collector
should therefore wait until no *new* device has been heard for a further
couple of seconds, and a long-running program should keep listening: a
charger switched on with shore power announces hours after any window.

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

## 3a. Device address and BusID

A device's address is fixed at the factory. MasterAdjust shows it as a 32-bit
**BusID**; the two are the same number:

```
BusID   = class << 24 | low18          (bits 18..23 always 0)
address = class << 18 | low18
```

**`class`** is a 5-bit product class, a constant in each firmware image.
Seen so far:

| class | devices |
|-------|---------|
| `0x06` | CombiMaster, Mass Combi Ultra, MCU |
| `0x0A` | ChargeMaster Plus, Mass Charger MB |
| `0x0C` | Alpha Pro III, SCM60 |
| `0x0E` | interfaces (MasterBus to NMEA, CANopen, LIN, …), repeater |
| `0x10` | Digital Input Switch, DC distribution (DSD, DCD) |
| `0x13` | MLI Ultra 1250 and MK3, lead battery monitor |
| `0x14` | displays (EasyView 5, SmartRemote); MasterAdjust itself |
| `0x18` | Mac Plus |
| `0x19` | ISO / GI transformer |
| `0x1B` | MLI Ultra cells, MasterShunt MKIII |
| `0x1D` | AC Power Analyser |

**`low18`** is computed from the device's 9-character serial `L Y WW L NNNN`
(e.g. `K712Q2000`) and its 8-digit article number:

```
y     = Y as a digit, with O = 10, N = 11, D = 12; then y & 3
A     = sum of the article number's 3-digit groups, from the right
        (77030500 -> 500 + 30 + 77)
low18 = (NNNN + (WW + 32*y) * 2048 + 64*A) & 0x3FFFF
```

The device recomputes it whenever its serial changes and stores it; the
letters are not used. This reproduces all 14 devices of the reference bus.

**A program on the bus has no serial**, so it makes up a BusID. MasterAdjust
uses class `0x14` with `low18` computed from the USB link's serial and
article `77030100`; the vendor's Linux library uses `0x500001` (BusID
`0x14000001`). Both send bus-master heartbeats (class `0x05`) from that
address. This crate uses class `0x1C`, which no firmware or bus seen so far
uses, with random `low18` kept in `config.ini` as `bus_id` (§7.3). Avoid
`0x1F` (the MLI MK3 treats it as a wildcard), `0x00` (unset to the
ChargeMaster Plus) and `0x00`–`0x09` (codes in firmware-update frames).

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
| `0x01` | `MonitoringValues`: the number of Monitoring values. |
| `0x02` | `MonitoringGroups`: the number of Monitoring groups (§2.1). |
| `0x03` | `HistoryValues`. |
| `0x04` | `HistoryGroups`. |
| `0x05` | `ConfigurationValues`. |
| `0x06` | `ConfigurationGroups`. Changes with the access level: a ChargeMaster Plus answers 11 at End User and 12 at Installer. |
| `0x07` | `AlarmsValues`. |
| `0x08` | `AlarmsGroups`. |
| `0x0D` | Named `MasterBusVoltage` in MasterAdjust (`FUN_00490b20`); a DSD answers `0x2C`, so the unit is not known. |
| `0x3E` | `EventGeneratingGroup` in MasterAdjust: the Configuration group that holds the device's events (`0x0A` on a ChargeMaster Plus, whose group `0x0A` lists its events). |
| `0x3F` | **Number of eventable outputs** (`Eventables` in MasterAdjust, which accepts at most 32). An ISO GI Trafo with one output (`Reset`) answers `1`. A device with none refuses it like `0x40 + K` below. Earlier versions of this document called it a total tab count, which it is not. |
| `0x40 + K` | Field index of the device's `K`-th **eventable output** (§9a), for `K` below the `0x3F` count. Past the last, a DSD answers the selector alone (`[0x08, SEL]`, DLC 2) from `addr \| 0x800000`. |

The names are MasterAdjust's (`Prod_GeneralToStr`, `00490b20`), as are the
others it knows: `0x19` UserLevel (§4.5), `0x3F` Eventables and `0x60`
MasterIndex. Earlier versions of this document read `0x03` as the number
of Configuration groups and `0x04` as Service groups; see §2.1. Other
selectors observed in MasterAdjust traces (`0x0A`–`0x0E`, `0x11`–`0x13`,
`0x1B`–`0x1F`, `0x23`) return values whose meaning hasn't been pinned down.

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

### 4.6 Flat field-index probe

A client can also probe a tab's whole field-index space: send the
visualization-type property (§6) for every index `0..256` and keep the
indices that answer. This finds fields no group lists, without their group.
MasterAdjust doesn't do it; the crate does it only when asked
(`Device::all_fields()`, `masterbus-dump --probe`), on the Monitoring and
Configuration tabs. Earlier versions relied on it for the Configuration
tab of devices whose `[0x08, 0x03]` was 0; `[0x08, 0x03]` is not a group
count (§2.1), and those devices list their settings in Configuration groups.

A login (§4.5) invalidates the flat-probe cache the same way it
invalidates the group cache: writability bits change per level and stale
attributes would lie to the caller.

---

## 5. Groups

Each tab's groups, numbered from 0 up to the tab's group count (§2.1), on
the tab's group classes. On the Monitoring tab that is class `0x19` to the
real address, answered on class `0x09`; the "not available" answer (class
`0x11` for Monitoring) carries only the request header. Result fields live
at `bytes[4:..]` of the answer.

| Purpose         | Request payload              | Answer value                           |
|-----------------|------------------------------|----------------------------------------|
| Group name id   | `[0x28, group_id, 0x00]`     | `u16LE` at bytes[4:6] (string id)      |
| Field count     | `[0x07, group_id, 0x00]`     | `f32LE` at bytes[4:8] (count)          |
| Field id        | `[0x03, group_id, 0x00, idx]`| `u16LE` at bytes[4:6] (field id)       |

The field-id response echoes `idx` in `bytes[3]` (used for matching, §8).
The field id is the field's index on the group's tab.

**An impossible field count.** A ChargeMaster Plus answered the field-count
query for some Monitoring group ids past its Monitoring count with 54,650,
consistently (#6); earlier versions read those as Configuration groups
(§2.1). Field-id
queries past its real fields answered those same fields again, so the count
is not a list of that many distinct fields. The field index in the request is
one byte, so a client can address at most 256 fields per group; that is a
limit of the request format, not something the device states. Such a group's
indices mostly answer the same two ids, its name is the article number, and
none of it appears in MasterAdjust's export. The crate asks for the count
again when it is over 256 and skips the group if it sticks.

**Late replies.** The same charger answers some schema and metadata requests
only after a client has given them up (three 150 ms attempts), sometimes all
three answers at once while the next request is out (#6). The crate keeps
listening for a field id it gave up on and collects it after the group's
other indices, and asks once more for metadata missing when its window
closes. A device that leaves out one metadata op for a field that answers
the others is not asked again for that op after two such misses. Group
field counts, option labels (`0x26`) and string chunks (`0x30`, §4.4) come
late the same way: a later chunk of a name can arrive after its request was
given up, which left "Outp" for "Output 2". The crate waits one more
timeout for a count or a chunk, since the next request depends on it, and
collects late option labels after the field's other options.

**"Not available" answers.** On the CombiMaster reference bus, group
queries past the Monitoring count came back on class `0x11`, Monitoring's
group "not available" answer, with a 4-byte payload `[opcode, group_id,
sub, value]`, e.g. `Up 11 188EA2 [4] 07 07 00 03`. Earlier versions asked
those as Configuration groups (§2.1). The crate takes such an answer to a
field count as a count of 0.

---

## 6. Per-field properties

Each field's name, unit, range and so on, asked on its tab's property
classes (§2.1), one opcode per request. Request payload
`[opcode, field_lo, field_hi]`; the answer echoes it and carries the value
at `bytes[4:..]`. On the Monitoring tab that is class `0x18` to
`addr | 0x800000`, answered on class `0x08` from the same address (the
high bit is what tells it from a Monitoring value); on the Configuration
tab, class `0x1C` to the real address, answered on class `0x0C`.

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

String ids returned by property queries are fetched with the same chunk
mechanism as §4.4, on class `0x07` to the real `device_addr`.

---

## 7. Values

### 7.1 Read a field value

Request: the tab's value class (§2.1), payload `[idx_lo, idx_hi]`. Answer:
`[idx_lo, idx_hi, b0, b1, b2, b3]` — a 4-byte value whose interpretation
depends on the field's visualization type. On the Monitoring tab that is
class `0x18` to the real address, answered on class `0x08`; on the
Configuration tab, class `0x1B` to `addr | 0x800000`, answered on class
`0x0B` from that address. A device sends a value only when asked, unless
another master is polling it. Earlier versions of this document called the
second byte a tab index; it is the index's high byte, 0 on every device seen
so far, and a nonzero one gets the "not available" answer.

### 7.2 Write a value

Writes use the **same class** as a read, but carry the field's full
**4-byte value** after the index. Every settable field — numeric,
boolean, or list — is written as the same 4-byte representation it is read back as:

```
[idx_lo, idx_hi, b0, b1, b2, b3]
```

Writes are seen on the wire for the Monitoring and Configuration tabs. A
Configuration write is answered with the new value; a Monitoring write
draws no answer, so the crate confirms it by reading it back. No write to
an Alarm or History field has been seen; the crate does not send one.

| Field type | 4-byte value                                     |
|------------|--------------------------------------------------|
| Float      | IEEE-754 single, little-endian                   |
| Boolean    | float `1.0` (`00 00 80 3f`) / `0.0` (`00 00 00 00`) |
| List/enum  | the selected index as a float (`1.0` = option 1) |

A **1-byte boolean write is ignored** by at least the CombiMaster — booleans must
be the full 4-byte float. The device confirms by pushing the new value (class
`0x08`); the immediate echo is unreliable, so confirm by re-reading.

### 7.3 Relay-control commit: naming the writer

Some controls do **not** act on the value write alone. After it, the writer
writes its own **BusID** (§3a) to an **adjacent hidden command register** at
`field_index + 1`:

```
[field_index + 1, 0x00, BusID >> 24, BusID & 0xFF, (BusID >> 8) & 0xFF, (BusID >> 16) & 0xFF]
```

Captured examples:

```
MasterAdjust, inverter on:   [0x13, 0x00, 00 00 80 3f]   then  [0x14, 0x00, 14 9f 3c 02]
EasyView event, stop charge: [0x27, 0x00, 00 00 80 3f]   then  [0x28, 0x00, 14 f9 94 02]
```

`14 9f 3c 02` is MasterAdjust's BusID `0x14023C9F`; `14 f9 94 02` is the
EasyView's own, `0x140294F9`. The command register is **write-only** in the
schema, but a device pushes its new value (the writer's BusID) as a class
`0x08` value. Without the commit write the value write is accepted but the
output does not change.

The device does **not** check the writer: a DSD acts on a commit naming a
BusID no device on the bus has, in any class (#29). It sends nothing to that
address.

A safe rule for emitting it: send the commit after a boolean write **only when
`field_index + 1` is not a real schema field**, so devices that don't use this
pattern are never disturbed. Float and list writes (e.g. an AC-input limit or a
drop-down) need no commit write. Never name a BusID a device on the bus has.

---

## 8. Request/response matching keys

Because responses carry no token, each in-flight request is keyed and matched
against incoming frames. `{tab}` is `monitoring`, `alarm`, `history` or
`configuration`, read from the answer's class and address half (§2.1);
`{addr}` is the real address.

| Exchange                | Key                                            |
|-------------------------|------------------------------------------------|
| Value                   | `val:{addr}:{field_id}`, the field id holding the tab |
| Property (0x06)         | `p:{addr}:{data[0]}:{data[1]}`; the 2-byte refusal of `[0x08, 0x40 + K]` from `addr \| 0x800000` matches the real address |
| String chunk (0x06,0x30)| `str:{addr}:{str_id}:{seq}` (seq from data[3]) |
| Group                   | `group:{tab}:{addr}:{op}:{group}` (+`:{idx}` for 0x03 field-id, idx in data[3]) |
| Field property          | `prop:{tab}:{addr}:{op}:{field}` (+`:{opt}` for 0x26, opt in data[3]) |

A "not available" answer is matched like the answer it stands for, so the
request resolves at once instead of timing out.

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
| `Event N data`    | 0x03 DropDown   | the action, one of exactly five: `Off` / `On` / `Copy` / `Copy invert` / `Toggle` |

The actions are carried out by the device firmware; MasterAdjust has no code
for them and only shows the device's labels. Its hidden "Check event list"
(`FUN_00637234`) requires the data field to have exactly 5 options, and every
event on the reference bus does. There is no action that sets a target to a
chosen value. An INT's string table also has `Pulse (1 sec)`, but no event
field on the reference bus offers it.

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
> fields — or ask the device for the list, as MasterAdjust does
> (`FUN_00687418`): `[0x08, 0x3F]` (§4.3) answers how many outputs there are
> (at most 32), and `[0x08, 0x40 + K]` the field index of output `K`, refused
> past the last. A DSD answers `03 05 07 09 0B 0D 0F 11 13 15 2E 31`
> (12 outputs), and the APR answers `0x43` with `0x27`, its 4th output.
> MasterAdjust asks `0x0D` (the eventable flag) only of Monitoring fields of
> viz 3, 4, 5 or 11 (`FUN_00681ed0`), which is why no other field is ever an
> output.
>
> **An output is not always on/off.** An EasyView's outputs (`State`,
> `Feed back 1`–`6`) are drop-downs `Off` / `On` / `Alarm`. An APR ALT 110A
> (article 45513000, firmware 4.0) reports its four outputs (`0x21` Bulk,
> `0x23` Absorption, `0x25` Float, `0x27` Stop charge) as **writable**
> (op `0x0B` = 1) drop-downs with a maximum of 3 and three options whose
> labels all name string 0, which is not a label (#29; MasterAdjust's export
> shows the same). The EasyView turns Stop charge on with `1.0` and off with
> `0.0`; what option 2 means is not known. What `On`, `Copy` or `Toggle`
> writes into such an output is up to the firmware.
>
> **MasterAdjust commits every write to an eventable field** (`FUN_006832d8`
> sets the commit flag from op `0x0D`), whatever its type: a drop-down is
> written as its option index, then committed (§7.3). masterbus commits only
> boolean writes, and switches an output with `1.0` (on) or `0.0` (off) plus
> the commit.
>
> An output's commit register is at its index + 1 (§7.3), so an output at
> index `0xFF` would have none: masterbus leaves such an output out of the
> list and does not switch it. No device has been seen to list one.

---

## 10. Typical enumeration sequence (per device)

1. Learn the device exists from a class-0x04 broadcast.
2. Firmware: `[0x82,0]`, `[0x82,1]` (§4.1).
3. Property string ids: `[0x09,1..4]` → fetch each string (§4.2, §4.4);
   derive revision from serial[4] if n=4 is unanswered.
4. For each tab (§2.1): its group count (§4.3).
5. For each group: name id, field count, field ids (§5); fetch the group name.
6. For each field: properties name/viz/max/unit (§6) on the field's tab;
   fetch name & unit strings; fetch option strings for list types.
7. (Optional) pre-poll each field's value (§7.1).
