//! Encode requests/writes into raw `(can_id, data)` pairs for transmission.

use super::{can_class, menu, meta_op, BTM1_META_ADDR_FLAG, TAB_DEFAULT};

fn id(class: u8, device_addr: u32) -> u32 {
    ((class as u32) << 24) | (device_addr & 0x00_FF_FF_FF)
}

/// Wire-level address transformation for Btm1 metadata requests. The Btm1
/// metadata channel addresses the device at `addr | 0x800000`; everything
/// else (Btm1 values, Btm3 metadata) uses the real address.
fn btm1_meta_addr(device_addr: u32) -> u32 {
    (device_addr | BTM1_META_ADDR_FLAG) & 0x00_FF_FF_FF
}

/// Bus-master heartbeat: class `0x05` from the master's own address, no data.
/// Devices announce (class `0x04`) in response, so emitting this lets us act as
/// the bus master and discover the bus when no hardware master is present.
pub fn heartbeat_raw(master_addr: u32) -> (u32, Vec<u8>) {
    (id(can_class::BUS_POLL, master_addr), Vec::new())
}

/// Read a monitoring field: class `0x18`, `[field, tab]`.
pub fn monitoring_req_raw(device_addr: u32, field_index: u8, tab_index: u8) -> (u32, Vec<u8>) {
    (id(can_class::MONITORING_REQ, device_addr), vec![field_index, tab_index])
}

/// Firmware-version query: class `0x07`, `[0x82, n]`.
pub fn fw_req_raw(device_addr: u32, n: u8) -> (u32, Vec<u8>) {
    (id(can_class::PROPERTY_REQ, device_addr), vec![0x82, n])
}

/// Group-count query for a menu selector: class `0x07`, `[0x08, selector]`.
pub fn group_count_req_raw(device_addr: u32, selector: u8) -> (u32, Vec<u8>) {
    let _ = menu::MONITORING; // selectors documented in `menu`
    (id(can_class::PROPERTY_REQ, device_addr), vec![0x08, selector])
}

/// Property string-id query: class `0x07`, `[0x09, n]`
/// (n: 1=article, 2=serial, 3=name, 4=revision).
pub fn prop_str_id_req_raw(device_addr: u32, n: u8) -> (u32, Vec<u8>) {
    (id(can_class::PROPERTY_REQ, device_addr), vec![0x09, n])
}

/// String-chunk query: class `0x07`, `[0x30, id_lo, id_hi, seq]`.
pub fn string_chunk_req_raw(device_addr: u32, str_id: u16, seq: u8) -> (u32, Vec<u8>) {
    let [lo, hi] = str_id.to_le_bytes();
    (id(can_class::PROPERTY_REQ, device_addr), vec![0x30, lo, hi, seq])
}

/// String-chunk write: class `0x07`, `[0x30, id_lo, id_hi, seq, c0..]` —
/// same opcode as [`string_chunk_req_raw`], but with up to 4 chars of payload
/// (`Dn` direction). A full 4-char chunk is 8 bytes; the terminator chunk
/// carries a single `0x00` (5 bytes) — even when the string is exactly N×4
/// chars, MasterAdjust emits an explicit NUL-terminator chunk.
pub fn string_chunk_write_raw(device_addr: u32, str_id: u16, seq: u8, chars: &[u8]) -> (u32, Vec<u8>) {
    debug_assert!(chars.len() <= 4, "string chunk carries at most 4 chars");
    let [lo, hi] = str_id.to_le_bytes();
    let mut data = vec![0x30, lo, hi, seq];
    data.extend_from_slice(chars);
    (id(can_class::PROPERTY_REQ, device_addr), data)
}

/// Schema group-name query: class `0x19`, `[0x28, group_id, 0x00]`.
pub fn schema_group_name_req_raw(device_addr: u32, group_id: u8) -> (u32, Vec<u8>) {
    (id(can_class::SCHEMA_REQ, device_addr), vec![0x28, group_id, 0x00])
}

/// Schema field-count query: class `0x19`, `[0x07, group_id, 0x00]`.
pub fn schema_field_count_req_raw(device_addr: u32, group_id: u8) -> (u32, Vec<u8>) {
    (id(can_class::SCHEMA_REQ, device_addr), vec![0x07, group_id, 0x00])
}

/// Schema field-id query: class `0x19`, `[0x03, group_id, 0x00, idx]`.
pub fn schema_field_id_req_raw(device_addr: u32, group_id: u8, idx: u8) -> (u32, Vec<u8>) {
    (id(can_class::SCHEMA_REQ, device_addr), vec![0x03, group_id, 0x00, idx])
}

/// Btm1 per-field metadata query: class `0x18` to the Btm1 metadata address
/// (`real | 0x800000`), payload `[opcode, field_lo, field_hi]`. The device
/// replies on class `0x08` at the same address.
pub fn btm1_meta_req_raw(device_addr: u32, opcode: u8, field_id: u16) -> (u32, Vec<u8>) {
    let [lo, hi] = field_id.to_le_bytes();
    (id(can_class::MONITORING_REQ, btm1_meta_addr(device_addr)), vec![opcode, lo, hi])
}

/// Btm1 option-string query: class `0x18` to the Btm1 metadata address,
/// payload `[0x26, field_id, 0x00, opt_idx]`.
pub fn btm1_meta_option_req_raw(device_addr: u32, field_id: u8, opt_idx: u8) -> (u32, Vec<u8>) {
    (
        id(can_class::MONITORING_REQ, btm1_meta_addr(device_addr)),
        vec![meta_op::OPTION, field_id, 0x00, opt_idx],
    )
}

/// Btm3 per-field metadata query: class `0x1C` to the device's **real**
/// address, payload `[opcode, field_lo, field_hi]`. Same opcode set as
/// [`btm1_meta_req_raw`]; the device replies on class `0x0C`. Used by
/// devices that speak the MasterBus v3 revision (e.g. the MacMagic
/// family) — see PROTOCOL.md §6.
pub fn btm3_meta_req_raw(device_addr: u32, opcode: u8, field_id: u16) -> (u32, Vec<u8>) {
    let [lo, hi] = field_id.to_le_bytes();
    (id(can_class::BTM3_META_REQ, device_addr), vec![opcode, lo, hi])
}

/// Btm3 option-string query: class `0x1C` to the real address, payload
/// `[0x26, field_id, 0x00, opt_idx]`.
pub fn btm3_meta_option_req_raw(device_addr: u32, field_id: u8, opt_idx: u8) -> (u32, Vec<u8>) {
    (
        id(can_class::BTM3_META_REQ, device_addr),
        vec![meta_op::OPTION, field_id, 0x00, opt_idx],
    )
}

/// Btm3 value read: class `0x1B` to the alternative address
/// (`real | 0x800000`), headerless 2-byte payload `[fid_lo, fid_hi]`. The
/// device replies on class `0x0B` at the same address with the current
/// 4-byte value. Btm3 value pushes are NOT autonomous on a quiet bus —
/// without an active read request the device never emits the value, so the
/// crate must drive each value-fetch itself.
pub fn btm3_read_raw(device_addr: u32, wire_idx: u8) -> (u32, Vec<u8>) {
    let [fid_lo, fid_hi] = (wire_idx as u16).to_le_bytes();
    (
        id(can_class::SCHEMA_REQ_HISTORY, btm1_meta_addr(device_addr)),
        vec![fid_lo, fid_hi],
    )
}

/// Btm3 write: class `0x1B` to the alternative address (`real | 0x800000`),
/// headerless payload `[fid_lo, fid_hi, b0, b1, b2, b3]` — a 16-bit wire
/// field index followed by a 4-byte little-endian `f32` value.
///
/// Every Btm3 write is 4 bytes, even when the field is a boolean or a list
/// pick: booleans go as `1.0` / `0.0`, list indices go as the index as
/// `f32`. (Same "everything's a float" rule as the Btm1 monitoring write
/// path; observed live in FINDINGS §3e.) The device acks by echoing the
/// new value on class `0x0B` at the same address.
pub fn btm3_write_raw(device_addr: u32, wire_idx: u8, value: f32) -> (u32, Vec<u8>) {
    let [fid_lo, fid_hi] = (wire_idx as u16).to_le_bytes();
    let [v0, v1, v2, v3] = value.to_le_bytes();
    (
        id(can_class::SCHEMA_REQ_HISTORY, btm1_meta_addr(device_addr)),
        vec![fid_lo, fid_hi, v0, v1, v2, v3],
    )
}

/// Schema group-name query on a non-default channel. `class` selects which
/// schema tab to query:
/// - [`can_class::SCHEMA_REQ`]         — Monitoring (default in [`schema_group_name_req_raw`])
/// - [`can_class::SCHEMA_REQ_ALARM`]   — Alarms
/// - [`can_class::SCHEMA_REQ_HISTORY`] — History / Service-events
///
/// Payload is the same `[0x28, group_id, 0x00]` regardless of channel; the
/// device replies on the matching response class (`0x09` / `0x0A` / `0x0B`).
pub fn schema_group_name_req_class_raw(class: u8, device_addr: u32, group_id: u8) -> (u32, Vec<u8>) {
    (id(class, device_addr), vec![0x28, group_id, 0x00])
}

/// Channel-specific schema field-count query (see [`schema_group_name_req_class_raw`]).
pub fn schema_field_count_req_class_raw(class: u8, device_addr: u32, group_id: u8) -> (u32, Vec<u8>) {
    (id(class, device_addr), vec![0x07, group_id, 0x00])
}

/// Channel-specific schema field-id query (see [`schema_group_name_req_class_raw`]).
pub fn schema_field_id_req_class_raw(
    class: u8,
    device_addr: u32,
    group_id: u8,
    idx: u8,
) -> (u32, Vec<u8>) {
    (id(class, device_addr), vec![0x03, group_id, 0x00, idx])
}

/// Boolean write: class `0x18`, `[field, tab, f32 LE]` with `1.0`=true / `0.0`=false.
/// Booleans go on the wire as the field's full 4-byte value (a `CheckBox` is just
/// a float that's 0 or 1), so a 1-byte write is ignored by e.g. the CombiMaster.
pub fn encode_set_boolean(device_addr: u32, field_index: u8, value: bool) -> (u32, Vec<u8>) {
    encode_set_float(device_addr, field_index, if value { 1.0 } else { 0.0 })
}

/// List/enum write: the selected index as a 4-byte float (e.g. option 1 -> `1.0`),
/// matching how the device reports it.
pub fn encode_set_list(device_addr: u32, field_index: u8, index: i32) -> (u32, Vec<u8>) {
    encode_set_float(device_addr, field_index, index as f32)
}

/// "Commit" token some relay-style controls (e.g. the CombiMaster inverter and
/// charger) require: after writing the value, write this fixed token to the
/// adjacent hidden command register (value field + 1) for the change to take
/// effect. Captured constant from MasterAdjust across inverter/charger, on/off.
pub const COMMIT_TOKEN: [u8; 4] = [0x14, 0x9f, 0x3c, 0x02];

/// Build the commit-token write for the command register `cmd_field`.
pub fn encode_commit(device_addr: u32, cmd_field: u8) -> (u32, Vec<u8>) {
    let mut data = vec![cmd_field, TAB_DEFAULT];
    data.extend_from_slice(&COMMIT_TOKEN);
    (id(can_class::MONITORING_REQ, device_addr), data)
}

/// Float write: class `0x18`, `[field, tab, f32 LE]`.
pub fn encode_set_float(device_addr: u32, field_index: u8, value: f32) -> (u32, Vec<u8>) {
    let mut data = vec![field_index, TAB_DEFAULT];
    data.extend_from_slice(&value.to_le_bytes());
    (id(can_class::MONITORING_REQ, device_addr), data)
}

/// The access-level register opcode (`0x08 0x19`) carried on class `0x07`
/// requests and `0x06` responses. See PROTOCOL.md §4.5.
pub const LOGIN_OPCODE: [u8; 2] = [0x08, 0x19];

/// Read the current access level: class `0x07`, `[0x08, 0x19]` (2 bytes).
/// The device replies with class `0x06`, `[0x08, 0x19, level, 0x00]`.
pub fn encode_login_read(device_addr: u32) -> (u32, Vec<u8>) {
    (id(can_class::PROPERTY_REQ, device_addr), LOGIN_OPCODE.to_vec())
}

/// Login: class `0x07`, `[0x08, 0x19, level, 0x00, f32 LE code]` (8 bytes).
/// The `level` byte must match the code (a level-2 frame carries `<redacted>`).
pub fn encode_login_write(device_addr: u32, level: u8, code: f32) -> (u32, Vec<u8>) {
    let mut data = vec![LOGIN_OPCODE[0], LOGIN_OPCODE[1], level, 0x00];
    data.extend_from_slice(&code.to_le_bytes());
    (id(can_class::PROPERTY_REQ, device_addr), data)
}

/// Logout (return to End User): class `0x07`, `[0x08, 0x19, 0x00, 0x00]`
/// (4 bytes — header only, no code value).
pub fn encode_logout(device_addr: u32) -> (u32, Vec<u8>) {
    (id(can_class::PROPERTY_REQ, device_addr), vec![LOGIN_OPCODE[0], LOGIN_OPCODE[1], 0x00, 0x00])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_class05_empty() {
        // Matches the observed master heartbeat on the wire: 0553A493# (no data).
        assert_eq!(heartbeat_raw(0x53A493), (0x0553A493, Vec::new()));
    }

    #[test]
    fn boolean_write_is_4byte_float() {
        // Matches MasterAdjust's inverter toggle: field, tab, float 1.0 / 0.0.
        assert_eq!(encode_set_boolean(0x188EA2, 0x13, true).1, vec![0x13, 0x00, 0x00, 0x00, 0x80, 0x3f]);
        assert_eq!(encode_set_boolean(0x188EA2, 0x13, false).1, vec![0x13, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn list_write_is_index_as_float() {
        // Matches MasterAdjust's dropdown write (Solar "Override" -> option 1 = 1.0).
        assert_eq!(encode_set_list(0x387028, 0x01, 1).1, vec![0x01, 0x00, 0x00, 0x00, 0x80, 0x3f]);
        assert_eq!(encode_set_list(0x188EA2, 0x05, 2).1, vec![0x05, 0x00, 0x00, 0x00, 0x00, 0x40]);
    }

    #[test]
    fn commit_token_is_fixed() {
        // CombiMaster inverter commit: field 0x14, tab 0, then the constant token.
        assert_eq!(encode_commit(0x188EA2, 0x14).1, vec![0x14, 0x00, 0x14, 0x9f, 0x3c, 0x02]);
    }

    #[test]
    fn requests_use_address_as_is() {
        // No address munging: e.g. a monitoring read of the CombiMaster (0x188EA2)
        // is class 0x18 to that exact address.
        assert_eq!(monitoring_req_raw(0x188EA2, 0x17, 0).0, 0x18188EA2);
        assert_eq!(monitoring_req_raw(0x6E96CF, 0, 0).0, 0x186E96CF);
    }

    #[test]
    fn btm3_write_matches_captured_wire_bytes() {
        // From FINDINGS §3e: ShutDown voltage (field 0x30) on the Nav Chg
        // (`0x3A3B4B`) written to 17.0 V went on the wire as
        //     `Dn 1B BA3B4B [6] 30 00 00 00 88 41`
        // class 0x1B, address `3A3B4B | 0x800000`, headerless `[fid_lo,
        // fid_hi, b0..b3]` with the f32 value little-endian.
        assert_eq!(
            btm3_write_raw(0x3A3B4B, 0x30, 17.0),
            (0x1B_BA3B4B, vec![0x30, 0x00, 0x00, 0x00, 0x88, 0x41]),
        );
        // Mode field 0x2C set to option index 2 ("Stabilizer") — list pick
        // as `f32(2.0)`, same as Btm1's "everything is a float" rule.
        assert_eq!(
            btm3_write_raw(0x3A3B4B, 0x2C, 2.0),
            (0x1B_BA3B4B, vec![0x2C, 0x00, 0x00, 0x00, 0x00, 0x40]),
        );
    }

    #[test]
    fn string_chunk_write_matches_captured_wire_bytes() {
        // From the Nav Chg (0x3A3B4B) Device-name edit at 2026-05-27T20:18:29,
        // changing the editable string at sid 0x0001 from "Nav Chg" to
        // "NavigationCh":
        //   Dn 07 3A3B4B [8]  30 01 00 00  4E 61 76 69    "Navi"
        //   Dn 07 3A3B4B [8]  30 01 00 01  67 61 74 69    "gati"
        //   Dn 07 3A3B4B [8]  30 01 00 02  6F 6E 43 68    "onCh"
        //   Dn 07 3A3B4B [5]  30 01 00 03  00             NUL terminator
        assert_eq!(
            string_chunk_write_raw(0x3A3B4B, 0x0001, 0, b"Navi"),
            (0x07_3A3B4B, vec![0x30, 0x01, 0x00, 0x00, 0x4E, 0x61, 0x76, 0x69]),
        );
        assert_eq!(
            string_chunk_write_raw(0x3A3B4B, 0x0001, 3, &[0x00]),
            (0x07_3A3B4B, vec![0x30, 0x01, 0x00, 0x03, 0x00]),
        );
    }

    #[test]
    fn login_frames_match_captured_wire_bytes() {
        // From the §3c / §3e captures (PROTOCOL.md §4.5).
        // Installer login on 0x188EA2 (<redacted>).
        assert_eq!(
            encode_login_write(0x188EA2, 0x01, <redacted>),
            (0x07_188EA2, vec![0x08, 0x19, 0x01, 0x00, <redacted>]),
        );
        // Distributor login on 0x53A493 (<redacted>).
        assert_eq!(
            encode_login_write(0x53A493, 0x02, <redacted>),
            (0x07_53A493, vec![0x08, 0x19, 0x02, 0x00, <redacted>]),
        );
        // Logout: 4 bytes, no code value.
        assert_eq!(encode_logout(0x43DF24), (0x07_43DF24, vec![0x08, 0x19, 0x00, 0x00]));
        // Read: 2 bytes.
        assert_eq!(encode_login_read(0x43DF24), (0x07_43DF24, vec![0x08, 0x19]));
    }
}
