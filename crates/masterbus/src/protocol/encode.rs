//! Encode requests/writes into raw `(can_id, data)` pairs for transmission.

use super::tab::{self, Kind};
use super::{TAB_DEFAULT, can_class, meta_op};
use crate::model::Menu;

fn id(class: u8, device_addr: u32) -> u32 {
    ((class as u32) << 24) | (device_addr & 0x00_FF_FF_FF)
}

/// Bus-master heartbeat: class `0x05` from the master's own address, no data.
/// Devices announce (class `0x04`) in response, so emitting this lets us act as
/// the bus master and discover the bus when no hardware master is present.
pub fn heartbeat_raw(master_addr: u32) -> (u32, Vec<u8>) {
    (id(can_class::BUS_POLL, master_addr), Vec::new())
}

/// A value read on `menu`'s tab: `[idx_lo, idx_hi]` (PROTOCOL.md §2.1). The
/// device answers `[idx_lo, idx_hi, b0..b3]`; it sends a value only when
/// asked, unless another master is polling it.
pub fn value_read_raw(menu: Menu, device_addr: u32, index: u8) -> (u32, Vec<u8>) {
    let e = tab::exchange(menu, Kind::Data);
    (e.req.can_id(device_addr), vec![index, 0x00])
}

/// A value write on `menu`'s tab: `[idx_lo, idx_hi, f32]`. The device
/// answers with the new value, as for a read.
pub fn value_write_raw(menu: Menu, device_addr: u32, index: u8, value: f32) -> (u32, Vec<u8>) {
    let e = tab::exchange(menu, Kind::Data);
    let mut data = vec![index, 0x00];
    data.extend_from_slice(&value.to_le_bytes());
    (e.req.can_id(device_addr), data)
}

/// Firmware-version query: class `0x07`, `[0x82, n]`.
pub fn fw_req_raw(device_addr: u32, n: u8) -> (u32, Vec<u8>) {
    (id(can_class::PROPERTY_REQ, device_addr), vec![0x82, n])
}

/// Property-by-selector query: class `0x07`, `[0x08, selector]`, the
/// selectors in [`super::menu`].
pub fn group_count_req_raw(device_addr: u32, selector: u8) -> (u32, Vec<u8>) {
    (
        id(can_class::PROPERTY_REQ, device_addr),
        vec![0x08, selector],
    )
}

/// Property string-id query: class `0x07`, `[0x09, n]`
/// (n: 1=article, 2=serial, 3=name, 4=revision).
pub fn prop_str_id_req_raw(device_addr: u32, n: u8) -> (u32, Vec<u8>) {
    (id(can_class::PROPERTY_REQ, device_addr), vec![0x09, n])
}

/// String-chunk query: class `0x07`, `[0x30, id_lo, id_hi, seq]`.
pub fn string_chunk_req_raw(device_addr: u32, str_id: u16, seq: u8) -> (u32, Vec<u8>) {
    let [lo, hi] = str_id.to_le_bytes();
    (
        id(can_class::PROPERTY_REQ, device_addr),
        vec![0x30, lo, hi, seq],
    )
}

/// String-chunk write: class `0x07`, `[0x30, id_lo, id_hi, seq, c0..]` —
/// same opcode as [`string_chunk_req_raw`], but with up to 4 chars of payload
/// (`Dn` direction). A full 4-char chunk is 8 bytes; the terminator chunk
/// carries a single `0x00` (5 bytes) — even when the string is exactly N×4
/// chars, MasterAdjust emits an explicit NUL-terminator chunk.
pub fn string_chunk_write_raw(
    device_addr: u32,
    str_id: u16,
    seq: u8,
    chars: &[u8],
) -> (u32, Vec<u8>) {
    debug_assert!(chars.len() <= 4, "string chunk carries at most 4 chars");
    let [lo, hi] = str_id.to_le_bytes();
    let mut data = vec![0x30, lo, hi, seq];
    data.extend_from_slice(chars);
    (id(can_class::PROPERTY_REQ, device_addr), data)
}

/// A group request on `menu`'s tab: `[op, group, 0x00]`, with `op` `0x07`
/// for the group's number of fields (`f32` at bytes 4..8 of the answer) or
/// `0x28` for its name's string id (`u16` at bytes 4..6) (PROTOCOL.md §5).
pub fn group_req_raw(menu: Menu, device_addr: u32, op: u8, group: u8) -> (u32, Vec<u8>) {
    let e = tab::exchange(menu, Kind::Group);
    (e.req.can_id(device_addr), vec![op, group, 0x00])
}

/// The field id at position `i` of a group on `menu`'s tab:
/// `[0x03, group, 0x00, i]`; the answer carries it as a `u16` at bytes 4..6.
pub fn group_field_id_req_raw(menu: Menu, device_addr: u32, group: u8, i: u8) -> (u32, Vec<u8>) {
    let e = tab::exchange(menu, Kind::Group);
    (e.req.can_id(device_addr), vec![0x03, group, 0x00, i])
}

/// A property of a field on `menu`'s tab: `[op, idx_lo, idx_hi]`, `op` one of
/// [`meta_op`] (PROTOCOL.md §6).
pub fn property_req_raw(menu: Menu, device_addr: u32, op: u8, index: u8) -> (u32, Vec<u8>) {
    let e = tab::exchange(menu, Kind::Property);
    (e.req.can_id(device_addr), vec![op, index, 0x00])
}

/// The label string id of option `opt` of a list field on `menu`'s tab:
/// `[0x26, idx_lo, idx_hi, opt]`.
pub fn property_option_req_raw(menu: Menu, device_addr: u32, index: u8, opt: u8) -> (u32, Vec<u8>) {
    let e = tab::exchange(menu, Kind::Property);
    (
        e.req.can_id(device_addr),
        vec![meta_op::OPTION, index, 0x00, opt],
    )
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

/// The commit-register value naming the writer `bus_id`: the BusID's top
/// byte, then its low three bytes least significant first (PROTOCOL.md §7.3).
/// A device acts on a relay-style write only once this follows it; the EasyView
/// commits with its own BusID, MasterAdjust with `14 9F 3C 02` (BusID
/// `0x14023C9F`).
pub fn commit_value(bus_id: u32) -> [u8; 4] {
    let b = bus_id.to_le_bytes();
    [b[3], b[0], b[1], b[2]]
}

/// The BusID of the device at CAN address `addr`: the address's class bits
/// (18..22) become the BusID's top byte, its low 18 bits stay (PROTOCOL.md §3a).
pub fn bus_id_for_addr(addr: u32) -> u32 {
    (((addr & 0x00FF_FFFF) >> 18) << 24) | (addr & 0x3FFFF)
}

/// The CAN address of the device with BusID `bus_id`; the inverse of
/// [`bus_id_for_addr`].
pub fn addr_for_bus_id(bus_id: u32) -> u32 {
    ((bus_id >> 24) << 18) | (bus_id & 0x3FFFF)
}

/// The commit write to the command register `cmd_field`, naming the writer
/// `bus_id`.
pub fn encode_commit(device_addr: u32, cmd_field: u8, bus_id: u32) -> (u32, Vec<u8>) {
    let mut data = vec![cmd_field, TAB_DEFAULT];
    data.extend_from_slice(&commit_value(bus_id));
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
    (
        id(can_class::PROPERTY_REQ, device_addr),
        LOGIN_OPCODE.to_vec(),
    )
}

/// Login: class `0x07`, `[0x08, 0x19, level, 0x00, f32 LE code]` (8 bytes).
/// The `level` byte and the f32 `code` are vendor-defined per device family;
/// this crate is opaque to their values and the caller supplies both.
pub fn encode_login_write(device_addr: u32, level: u8, code: f32) -> (u32, Vec<u8>) {
    let mut data = vec![LOGIN_OPCODE[0], LOGIN_OPCODE[1], level, 0x00];
    data.extend_from_slice(&code.to_le_bytes());
    (id(can_class::PROPERTY_REQ, device_addr), data)
}

/// Logout (return to End User): class `0x07`, `[0x08, 0x19, 0x00, 0x00]`
/// (4 bytes — header only, no code value).
pub fn encode_logout(device_addr: u32) -> (u32, Vec<u8>) {
    (
        id(can_class::PROPERTY_REQ, device_addr),
        vec![LOGIN_OPCODE[0], LOGIN_OPCODE[1], 0x00, 0x00],
    )
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
        assert_eq!(
            encode_set_boolean(0x188EA2, 0x13, true).1,
            vec![0x13, 0x00, 0x00, 0x00, 0x80, 0x3f]
        );
        assert_eq!(
            encode_set_boolean(0x188EA2, 0x13, false).1,
            vec![0x13, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn list_write_is_index_as_float() {
        // Matches MasterAdjust's dropdown write (Solar "Override" -> option 1 = 1.0).
        assert_eq!(
            encode_set_list(0x387028, 0x01, 1).1,
            vec![0x01, 0x00, 0x00, 0x00, 0x80, 0x3f]
        );
        assert_eq!(
            encode_set_list(0x188EA2, 0x05, 2).1,
            vec![0x05, 0x00, 0x00, 0x00, 0x00, 0x40]
        );
    }

    #[test]
    fn commit_value_names_the_writer() {
        // From the #29 capture: the EasyView (BusID 0x140294F9, CAN 0x5294F9)
        // commits with 14 F9 94 02; the APR (0x0C0193BA, CAN 0x3193BA) with
        // 0C BA 93 01. MasterAdjust's constant 14 9F 3C 02 is BusID 0x14023C9F.
        assert_eq!(commit_value(0x140294F9), [0x14, 0xF9, 0x94, 0x02]);
        assert_eq!(commit_value(0x0C0193BA), [0x0C, 0xBA, 0x93, 0x01]);
        assert_eq!(commit_value(0x14023C9F), [0x14, 0x9F, 0x3C, 0x02]);
        assert_eq!(
            encode_commit(0x3193BA, 0x28, 0x140294F9).1,
            vec![0x28, 0x00, 0x14, 0xF9, 0x94, 0x02]
        );
    }

    #[test]
    fn bus_id_and_address_are_the_same_number() {
        assert_eq!(bus_id_for_addr(0x5294F9), 0x140294F9);
        assert_eq!(bus_id_for_addr(0x3193BA), 0x0C0193BA);
        assert_eq!(addr_for_bus_id(0x140294F9), 0x5294F9);
        assert_eq!(addr_for_bus_id(0x1C03FFFF), 0x73FFFF);
        for addr in [0x000001, 0x188EA2, 0x6CC699, 0x7FFFFF] {
            assert_eq!(addr_for_bus_id(bus_id_for_addr(addr)), addr);
        }
    }

    #[test]
    fn requests_use_address_as_is() {
        // No address munging: e.g. a monitoring read of the CombiMaster (0x188EA2)
        // is class 0x18 to that exact address.
        let read = |addr, i| value_read_raw(Menu::Monitoring, addr, i);
        assert_eq!(read(0x188EA2, 0x17), (0x18188EA2, vec![0x17, 0x00]));
        assert_eq!(read(0x6E96CF, 0).0, 0x186E96CF);
    }

    #[test]
    fn a_configuration_write_matches_captured_wire_bytes() {
        // From FINDINGS §3e: ShutDown voltage (field 0x30) on the Nav Chg
        // (`0x3A3B4B`) written to 17.0 V went on the wire as
        //     `Dn 1B BA3B4B [6] 30 00 00 00 88 41`
        // class 0x1B, address `3A3B4B | 0x800000`, headerless `[fid_lo,
        // fid_hi, b0..b3]` with the f32 value little-endian.
        assert_eq!(
            value_write_raw(Menu::Configuration, 0x3A3B4B, 0x30, 17.0),
            (0x1B_BA3B4B, vec![0x30, 0x00, 0x00, 0x00, 0x88, 0x41]),
        );
        // Mode field 0x2C set to option index 2 ("Stabilizer") — list pick
        // as `f32(2.0)`, same as Btm1's "everything is a float" rule.
        assert_eq!(
            value_write_raw(Menu::Configuration, 0x3A3B4B, 0x2C, 2.0),
            (0x1B_BA3B4B, vec![0x2C, 0x00, 0x00, 0x00, 0x00, 0x40]),
        );
    }

    /// The requests MasterAdjust sends a ChargeMaster Plus (#6).
    #[test]
    fn tab_requests_match_masteradjusts_trace() {
        const CHG: u32 = 0x2A42F5;
        assert_eq!(
            group_req_raw(Menu::Configuration, CHG, 0x07, 0x0A),
            (0x1C_AA42F5, vec![0x07, 0x0A, 0x00])
        );
        assert_eq!(
            group_field_id_req_raw(Menu::Configuration, CHG, 0x00, 0x03),
            (0x1C_AA42F5, vec![0x03, 0x00, 0x00, 0x03])
        );
        assert_eq!(
            property_req_raw(Menu::Configuration, CHG, 0x02, 0x06),
            (0x1C_2A42F5, vec![0x02, 0x06, 0x00])
        );
        assert_eq!(
            value_read_raw(Menu::Configuration, CHG, 0x02),
            (0x1B_AA42F5, vec![0x02, 0x00])
        );
        assert_eq!(
            property_req_raw(Menu::Alarm, CHG, 0x02, 0x01),
            (0x19_AA42F5, vec![0x02, 0x01, 0x00])
        );
        assert_eq!(
            value_read_raw(Menu::Alarm, CHG, 0x11),
            (0x03_2A42F5, vec![0x11, 0x00])
        );
        assert_eq!(
            group_req_raw(Menu::History, CHG, 0x07, 0x05),
            (0x1B_2A42F5, vec![0x07, 0x05, 0x00])
        );
        assert_eq!(
            property_req_raw(Menu::History, CHG, 0x2C, 0x06),
            (0x1A_AA42F5, vec![0x2C, 0x06, 0x00])
        );
        assert_eq!(
            value_read_raw(Menu::History, CHG, 0x06),
            (0x1D_AA42F5, vec![0x06, 0x00])
        );
        assert_eq!(
            property_option_req_raw(Menu::Monitoring, CHG, 0x08, 0x01),
            (0x18_AA42F5, vec![0x26, 0x08, 0x00, 0x01])
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
            (
                0x07_3A3B4B,
                vec![0x30, 0x01, 0x00, 0x00, 0x4E, 0x61, 0x76, 0x69]
            ),
        );
        assert_eq!(
            string_chunk_write_raw(0x3A3B4B, 0x0001, 3, &[0x00]),
            (0x07_3A3B4B, vec![0x30, 0x01, 0x00, 0x03, 0x00]),
        );
    }

    #[test]
    fn login_frames_match_captured_wire_bytes() {
        // The encoder is opaque to the value of `code`: any f32 round-trips
        // into the wire payload regardless of which level it ends up
        // unlocking on a given device family. A dummy `0.0` exercises the
        // layout without naming any vendor-specific code.
        assert_eq!(
            encode_login_write(0x188EA2, 0x01, 0.0),
            (
                0x07_188EA2,
                vec![0x08, 0x19, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]
            ),
        );
        // Logout: 4 bytes, no code value.
        assert_eq!(
            encode_logout(0x43DF24),
            (0x07_43DF24, vec![0x08, 0x19, 0x00, 0x00])
        );
        // Read: 2 bytes.
        assert_eq!(encode_login_read(0x43DF24), (0x07_43DF24, vec![0x08, 0x19]));
    }
}
