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

| Class | Name             | Dir | Meaning                                                   |
|-------|------------------|-----|-----------------------------------------------------------|
| 0x04  | DEVICE_BROADCAST | ←   | Periodic 8-byte device self-announcement                  |
| 0x05  | BUS_POLL         | ←   | Bus-master heartbeat (0-byte frame from node `0x500001`)  |
| 0x06  | PROPERTY_INFO    | ←   | Property / group-count response from a device             |
| 0x07  | PROPERTY_REQ     | →   | Property / group-count request to a device                |
| 0x08  | MONITORING_DATA  | ←   | Monitoring value pushed from a device (also shadow resp.) |
| 0x09  | SCHEMA_DATA      | ←   | Schema response from a device                             |
| 0x18  | MONITORING_REQ   | →   | Monitoring request to a device (also shadow request)      |
| 0x19  | SCHEMA_REQ       | →   | Schema request to a device                                |

`→` = controller→device (request), `←` = device→controller (response/push).

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
[4:6] firmware_version  u16 little-endian (basic)
[6:8] (unidentified)
```

The 24-bit `device_addr` comes from the CAN ID, not the payload.

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

### 4.3 Monitoring group count — `[0x08, 0x02]`

Response `[0x08, 0x02, count_lo, count_hi]` → `u16LE` = number of **monitoring**
groups.

> The frequently-seen query `[0x08, 0x3F]` returns the **total tab count**
> (Monitoring + Alarm + History + Configuration), *not* the monitoring-group
> count. Use `[0x08, 0x02]` for monitoring groups.

### 4.4 String-table chunk — `[0x30, id_lo, id_hi, seq]`

Fetches a string by id, 4 characters per chunk, starting at `seq = 0`.

Response: `[0x30, id_lo, id_hi, seq, c0, c1, c2, c3]` (chars truncated on the
last chunk).

Termination: stop when a chunk contains a `NUL` (0x00) byte **or** is shorter
than 8 bytes (fewer than 4 chars). Otherwise request `seq + 1`.

---

## 5. Schema queries (class 0x19 request → 0x09 response)

Per-group structure. Requests are class `0x19` to `device_addr`; responses class
`0x09`. Result fields live at `bytes[4:..]` of the response.

| Purpose         | Request payload              | Response value                         |
|-----------------|------------------------------|----------------------------------------|
| Group name id   | `[0x28, group_id, 0x00]`     | `u16LE` at bytes[4:6] (string id)      |
| Field count     | `[0x07, group_id, 0x00]`     | `f32LE` at bytes[4:8] (count)          |
| Field id        | `[0x03, group_id, 0x00, idx]`| `u16LE` at bytes[4:6] (field id)       |

The field-id response echoes `idx` in `bytes[3]` (used for matching, §8).

---

## 6. Shadow (field) metadata

Per-field metadata is requested at the device's **shadow address**:

```
shadow_addr = (device_addr | 0x800000) & 0x00FF_FFFF
```

Request: class `0x18` to `shadow_addr`, payload `[opcode, field_lo, field_hi]`.
Response: class `0x08` from `shadow_addr` (the high bit `0x800000` distinguishes
a shadow response from a real monitoring push).

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
- **A shadow string id of `0` means "no string"** — the opposite of property
  string ids (§4.2). Unitless fields often return no `0x2C` response at all.
- Option strings (`0x26`) are only present for list-type fields and only up to
  `option count` (`0x07`).
- Opcodes `0x02/0x07/0x26/0x28/0x2C` are confirmed on the wire from live capture;
  the others (`0x03/0x06/0x08/0x09/0x0B/0x0C/0x0D/0x29/0x2A`) are from the
  MasterAdjust decompile and should be confirmed live when used.

String ids returned by shadow queries are fetched with the same chunk mechanism
as §4.4 (to the real `device_addr`, not the shadow address).

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
| Shadow (0x08, hi bit)   | `shadow:{real_addr}:{op}:{field}` (+`:{opt}` for 0x26, opt in data[3]) |

Regular monitoring pushes (class 0x08, no `0x800000` bit) are routed to the
value table; shadow responses (class 0x08 with the `0x800000` address bit) are
routed to the metadata waiter.

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
| 0x07      | Time          |
| 0x08      | Date          |

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

### Eventable / DeviceList
`byte[0]` = selected index (not yet re-checked against the float form above —
likely also a float, given Radio/DropDown).

### Text
UTF-8 bytes of the payload (lossy decode).

---

## 10. Typical enumeration sequence (per device)

1. Learn the device exists from a class-0x04 broadcast.
2. Firmware: `[0x82,0]`, `[0x82,1]` (§4.1).
3. Property string ids: `[0x09,1..4]` → fetch each string (§4.2, §4.4);
   derive revision from serial[4] if n=4 is unanswered.
4. Monitoring group count: `[0x08,0x02]` (§4.3).
5. For each group: name id, field count, field ids (§5); fetch the group name.
6. For each field: shadow name/viz/max/unit (§6); fetch name & unit strings;
   fetch option strings for list types.
7. (Optional) pre-poll each field's value (§7.1).
