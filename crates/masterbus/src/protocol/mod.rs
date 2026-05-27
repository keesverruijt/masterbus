//! MasterBus wire protocol: frame types, class constants, and codec.
//!
//! All frames are 29-bit **extended** CAN frames whose id is
//! `(can_class << 24) | device_addr`. See `docs/PROTOCOL.md`.

mod decode;
mod encode;

pub use decode::{decode_value, frame_from_raw, parse_frame, waiter_key_for_frame};
pub use encode::{
    encode_commit, encode_login_read, encode_login_write, encode_logout, encode_set_boolean,
    encode_set_float, encode_set_list, fw_req_raw, group_count_req_raw, heartbeat_raw,
    monitoring_req_raw, prop_str_id_req_raw, schema_field_count_req_class_raw,
    schema_field_count_req_raw, schema_field_id_req_class_raw, schema_field_id_req_raw,
    schema_group_name_req_class_raw, schema_group_name_req_raw, shadow_meta_req_magic_raw,
    shadow_meta_req_raw, shadow_option_req_magic_raw, shadow_option_req_raw,
    string_chunk_req_raw, LOGIN_OPCODE,
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
    /// Monitoring/shadow data pushed from a device.
    pub const MONITORING_DATA: u8 = 0x08;
    /// Schema response (Monitoring channel) from a device.
    pub const SCHEMA_DATA: u8 = 0x09;
    /// Schema response — Alarms tab schema channel (paired with [`SCHEMA_REQ_ALARM`]).
    pub const SCHEMA_DATA_ALARM: u8 = 0x0A;
    /// Dual-purpose class:
    /// - On the **real** address: schema response for the History/Service-events
    ///   tab (paired with [`SCHEMA_REQ_HISTORY`]).
    /// - On the **shadow** address (`addr | 0x800000`): a headerless live-value
    ///   push `[fid_lo, fid_hi, b0..b3]` used by Magic-class devices to
    ///   broadcast field values without a `class 0x18` request.
    pub const SCHEMA_DATA_HISTORY: u8 = 0x0B;
    /// Magic-class shadow metadata response on the **real** address
    /// (paired with [`SHADOW_REQ_MAGIC`]). Same payload layout as a standard
    /// shadow response on the shadow address.
    pub const SHADOW_DATA_MAGIC: u8 = 0x0C;
    /// Write/no-value acknowledgement from a device (`[field, tab]`).
    pub const WRITE_ACK: u8 = 0x10;
    /// Compact / "not available" schema response (4-byte). Sent when a
    /// schema query targets a tab+gid that exists only via a sibling channel
    /// (e.g. Config gids 7-9 on a CombiMaster respond with this on class 0x19).
    pub const SCHEMA_DATA_NA: u8 = 0x11;
    /// Monitoring/shadow request to a device (also a device push when 6 bytes).
    pub const MONITORING_REQ: u8 = 0x18;
    /// Schema request — Monitoring tab.
    pub const SCHEMA_REQ: u8 = 0x19;
    /// Schema request — Alarms tab.
    pub const SCHEMA_REQ_ALARM: u8 = 0x1A;
    /// Schema request — History / Service-events tab.
    pub const SCHEMA_REQ_HISTORY: u8 = 0x1B;
    /// Shadow metadata request — Magic-class channel (real address). Same
    /// opcode set as the standard `0x18` shadow request, but sent to the
    /// device's real address; the device replies on [`SHADOW_DATA_MAGIC`].
    pub const SHADOW_REQ_MAGIC: u8 = 0x1C;
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

/// Per-field metadata opcodes (shadow address `device_addr | 0x800000`).
pub mod shadow_op {
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

/// Shadow-address bit set on the device address for per-field metadata.
pub const SHADOW_BIT: u32 = 0x80_0000;

/// How a field is presented/edited, decoded from [`shadow_op::VIZ`].
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

/// Map the wire visualization byte (shadow op `0x02`) to a [`VisualizationType`].
pub fn viz_from_wire(code: u8) -> VisualizationType {
    match code {
        0x01 => VisualizationType::Float,
        0x03 => VisualizationType::DropDown,
        0x04 => VisualizationType::Eventable,
        0x05 => VisualizationType::CheckBox,
        0x07 => VisualizationType::Time,
        0x08 => VisualizationType::Date,
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
