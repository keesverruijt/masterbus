//! Encode requests/writes into raw `(can_id, data)` pairs for transmission.

use super::{can_class, menu, shadow_op, SHADOW_BIT, TAB_DEFAULT};

fn id(class: u8, device_addr: u32) -> u32 {
    ((class as u32) << 24) | (device_addr & 0x00_FF_FF_FF)
}

fn shadow_addr(device_addr: u32) -> u32 {
    (device_addr | SHADOW_BIT) & 0x00_FF_FF_FF
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

/// Shadow metadata query: class `0x18` to the shadow address, `[opcode, field_lo, field_hi]`.
pub fn shadow_meta_req_raw(device_addr: u32, opcode: u8, field_id: u16) -> (u32, Vec<u8>) {
    let [lo, hi] = field_id.to_le_bytes();
    (id(can_class::MONITORING_REQ, shadow_addr(device_addr)), vec![opcode, lo, hi])
}

/// Shadow option-string query: class `0x18` to the shadow address,
/// `[0x26, field_id, 0x00, opt_idx]`.
pub fn shadow_option_req_raw(device_addr: u32, field_id: u8, opt_idx: u8) -> (u32, Vec<u8>) {
    (
        id(can_class::MONITORING_REQ, shadow_addr(device_addr)),
        vec![shadow_op::OPTION, field_id, 0x00, opt_idx],
    )
}

/// Boolean write: class `0x18`, `[field, tab, 0|1]`.
pub fn encode_set_boolean(device_addr: u32, field_index: u8, value: bool) -> (u32, Vec<u8>) {
    (
        id(can_class::MONITORING_REQ, device_addr),
        vec![field_index, TAB_DEFAULT, value as u8],
    )
}

/// Float write: class `0x18`, `[field, tab, f32 LE]`.
pub fn encode_set_float(device_addr: u32, field_index: u8, value: f32) -> (u32, Vec<u8>) {
    let mut data = vec![field_index, TAB_DEFAULT];
    data.extend_from_slice(&value.to_le_bytes());
    (id(can_class::MONITORING_REQ, device_addr), data)
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
    fn requests_use_address_as_is() {
        // No address munging: e.g. a monitoring read of the CombiMaster (0x188EA2)
        // is class 0x18 to that exact address.
        assert_eq!(monitoring_req_raw(0x188EA2, 0x17, 0).0, 0x18188EA2);
        assert_eq!(monitoring_req_raw(0x6E96CF, 0, 0).0, 0x186E96CF);
    }
}
