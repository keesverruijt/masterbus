//! MasterBus wire protocol: frame types, class constants, and codec.
//!
//! All frames are 29-bit **extended** CAN frames whose id is
//! `(can_class << 24) | device_addr`. See `docs/PROTOCOL.md`.

mod decode;
mod encode;

pub use decode::{decode_value, frame_from_raw, parse_frame, waiter_key_for_frame};
pub use encode::{
    LOGIN_OPCODE, btm1_meta_option_req_raw, btm1_meta_req_raw, btm3_meta_option_req_raw,
    btm3_meta_req_raw, btm3_read_raw, btm3_write_raw, encode_commit, encode_login_read,
    encode_login_write, encode_logout, encode_set_boolean, encode_set_float, encode_set_list,
    fw_req_raw, group_count_req_raw, heartbeat_raw, monitoring_req_raw, prop_str_id_req_raw,
    schema_field_count_req_class_raw, schema_field_count_req_raw, schema_field_id_req_class_raw,
    schema_field_id_req_raw, schema_group_name_req_class_raw, schema_group_name_req_raw,
    string_chunk_req_raw, string_chunk_write_raw,
};

/// CAN class bytes (bits 28:24 of the 29-bit id).
pub mod can_class {
    /// Device status broadcast — periodic self-announcement (liveness).
    pub const DEVICE_BROADCAST: u8 = 0x04;
    /// Bus-master heartbeat (0-byte frame from the master node).
    pub const BUS_POLL: u8 = 0x05;
    /// Property/count response from a device.
    pub const PROPERTY_INFO: u8 = 0x06;
    /// Property/count request to a device.
    pub const PROPERTY_REQ: u8 = 0x07;
    /// Btm1 monitoring/metadata data pushed from a device.
    pub const MONITORING_DATA: u8 = 0x08;
    /// Schema response (Monitoring channel) from a device.
    pub const SCHEMA_DATA: u8 = 0x09;
    /// Schema response — Alarms-tab schema channel (paired with [`SCHEMA_REQ_ALARM`]).
    pub const SCHEMA_DATA_ALARM: u8 = 0x0A;
    /// Dual-purpose class:
    /// - On the device's real address: schema response for the
    ///   History/Service-events tab (paired with [`SCHEMA_REQ_HISTORY`]).
    /// - On `addr | 0x800000`: headerless **Btm3** live-value carrier
    ///   `[fid_lo, fid_hi, b0..b3]` — both unsolicited pushes and
    ///   write-acks land here.
    pub const SCHEMA_DATA_HISTORY: u8 = 0x0B;
    /// Btm3 metadata response on the real address (paired with
    /// [`BTM3_META_REQ`]). Same payload layout as a Btm1 metadata response.
    pub const BTM3_META_DATA: u8 = 0x0C;
    /// Write/no-value acknowledgement from a device (`[field, tab]`).
    pub const WRITE_ACK: u8 = 0x10;
    /// Compact / "not available" schema response (4-byte). Sent when a
    /// schema query targets a tab+gid that exists only via a sibling channel
    /// (e.g. Config gids 7-9 on a CombiMaster respond with this on class 0x19).
    pub const SCHEMA_DATA_NA: u8 = 0x11;
    /// Btm1 monitoring / metadata request to a device. Also seen as a
    /// device push when 6 bytes long (a value carrier).
    pub const MONITORING_REQ: u8 = 0x18;
    /// Schema request — Monitoring tab.
    pub const SCHEMA_REQ: u8 = 0x19;
    /// Schema request — Alarms tab.
    pub const SCHEMA_REQ_ALARM: u8 = 0x1A;
    /// Schema request — History / Service-events tab (TBD: may overlap with
    /// the Btm3 write class on `addr | 0x800000`; see protocol notes).
    pub const SCHEMA_REQ_HISTORY: u8 = 0x1B;
    /// Btm3 metadata request: same opcode set as [`MONITORING_REQ`] but
    /// addressed to the device's real address. The device replies on
    /// [`BTM3_META_DATA`].
    pub const BTM3_META_REQ: u8 = 0x1C;
}

/// Tab byte used in monitoring requests. The field index is global; the tab byte
/// is always 0 (a nonzero tab returns a class-`0x10` "no value").
pub const TAB_DEFAULT: u8 = 0x00;

/// Per-menu group-count selectors for the `[0x07] [0x08, selector]` query.
pub mod menu {
    /// Monitoring groups.
    pub const MONITORING: u8 = 0x02;
    /// Configuration / installer groups.
    pub const CONFIG: u8 = 0x03;
    /// Service-level groups.
    pub const SERVICE: u8 = 0x04;
    /// Total group count (all menus).
    pub const TOTAL: u8 = 0x3F;
}

/// Per-field metadata opcodes carried inside Btm1 and Btm3 metadata frames.
/// Same opcode set on both channels; the channel difference is in the CAN
/// class + address (see [`can_class::MONITORING_REQ`] / [`can_class::BTM3_META_REQ`]).
pub mod meta_op {
    /// Field name string id.
    pub const NAME: u8 = 0x28;
    /// Visualization type byte.
    pub const VIZ: u8 = 0x02;
    /// Minimum (f32).
    pub const MIN: u8 = 0x06;
    /// Maximum / option count (f32).
    pub const MAX: u8 = 0x07;
    /// Step (f32).
    pub const STEP: u8 = 0x08;
    /// Factory default (f32).
    pub const FACTORY_DEFAULT: u8 = 0x09;
    /// Writeable flag (`byte[4]`: 1 = writable).
    pub const WRITEABLE: u8 = 0x0B;
    /// Unit string id.
    pub const UNIT: u8 = 0x2C;
    /// Option string id (`[0x26, field, 0x00, opt_idx]`).
    pub const OPTION: u8 = 0x26;
}

/// Address flag bit set on the device address to request Btm1 per-field
/// metadata (and to identify Btm3 value pushes). Wire-level only; not part
/// of the public field-id space.
pub(crate) const BTM1_META_ADDR_FLAG: u32 = 0x80_0000;

/// How a field is presented/edited, decoded from [`meta_op::VIZ`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VisualizationType {
    /// Numeric (editable).
    Float,
    /// Numeric, greyed/read-only display.
    GrayVisualization,
    /// Checkbox boolean.
    CheckBox,
    /// Toggle button boolean.
    ToggleButton,
    /// Push button boolean.
    PushButton,
    /// Clock/duration.
    Time,
    /// Calendar date.
    Date,
    /// Radio list.
    Radio,
    /// Drop-down list.
    DropDown,
    /// Eventable list.
    Eventable,
    /// Device reference list.
    DeviceList,
    /// Free text.
    Text,
}

/// Map the wire visualization byte (meta op `0x02`) to a [`VisualizationType`].
///
/// The full code set was recovered by cross-referencing MasterAdjust's captured
/// `VisualizationType` field (which equals the wire code) against known field
/// types: `0x01`/`0x03`/`0x06`/`0x07`/`0x08` line up exactly on Float / DropDown
/// / Text / Time / Date. `0x09` (`DeviceList`) is the event **target** — an
/// index into the address-sorted bus device list — and was previously
/// unmapped, so event-target fields rendered as a raw number. See FINDINGS.
pub fn viz_from_wire(code: u8) -> VisualizationType {
    match code {
        0x01 => VisualizationType::Float,
        0x03 => VisualizationType::DropDown,
        0x04 => VisualizationType::Eventable,
        0x05 => VisualizationType::CheckBox,
        0x06 => VisualizationType::Text,
        0x07 => VisualizationType::Time,
        0x08 => VisualizationType::Date,
        // Event target: a device reference (index into the sorted device list).
        0x09 => VisualizationType::DeviceList,
        // 0x0A is the event "command" (which of the target's eventable outputs);
        // resolving its label needs the target's eventable list, not yet
        // reversed, so it stays a plain index for now.
        _ => VisualizationType::Float,
    }
}

/// A decoded extended CAN frame.
#[derive(Debug, Clone)]
pub struct MbFrame {
    /// 24-bit device address (bits 23:0).
    pub device_addr: u32,
    /// Class byte (bits 28:24).
    pub can_class: u8,
    /// 0–8 payload bytes.
    pub data: Vec<u8>,
}

impl MbFrame {
    /// Reconstruct the 29-bit CAN id.
    pub fn can_id(&self) -> u32 {
        ((self.can_class as u32) << 24) | (self.device_addr & 0x00_FF_FF_FF)
    }
}

/// A parsed protocol message.
#[derive(Debug, Clone)]
pub enum MbMessage {
    /// Periodic device self-announcement (class 0x04).
    DeviceBroadcast {
        /// Device address.
        device_addr: u32,
        /// Device family/type code.
        type_code: u8,
        /// Sub-device instance.
        instance: u8,
        /// Firmware version (u16 LE).
        firmware_version: u16,
    },
    /// A value request (`[field, tab]`).
    MonitoringReq {
        /// Device address.
        device_addr: u32,
        /// Field index.
        field_index: u8,
        /// Tab byte.
        tab_index: u8,
    },
    /// A value push/response (`[field, tab, value(4)]`).
    MonitoringData {
        /// Device address.
        device_addr: u32,
        /// Field index.
        field_index: u8,
        /// Tab byte.
        tab_index: u8,
        /// Raw value bytes.
        raw: Vec<u8>,
    },
    /// Anything not recognised.
    Unknown {
        /// Device address.
        device_addr: u32,
        /// Class byte.
        can_class: u8,
        /// Raw payload.
        data: Vec<u8>,
    },
}
