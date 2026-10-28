//! MasterBus wire protocol: frame types, class constants, and codec.
//!
//! All frames are 29-bit **extended** CAN frames whose id is
//! `(can_class << 24) | device_addr`. See `docs/PROTOCOL.md`.

mod decode;
mod encode;
pub mod tab;

pub use decode::{decode_value, frame_from_raw, parse_frame, waiter_key_for_frame};
pub use encode::{
    LOGIN_OPCODE, addr_for_bus_id, bus_id_for_addr, commit_value, encode_commit, encode_login_read,
    encode_login_write, encode_logout, encode_set_boolean, encode_set_float, encode_set_list,
    fw_req_raw, group_count_req_raw, group_field_id_req_raw, group_req_raw, heartbeat_raw,
    prop_str_id_req_raw, property_option_req_raw, property_req_raw, string_chunk_req_raw,
    string_chunk_write_raw, value_read_raw, value_write_raw,
};

/// CAN class bytes (bits 28:24 of the 29-bit id). The classes of the four
/// tabs' values, properties and groups are in [`tab`]; the names here are
/// those classes' uses on the Monitoring tab, kept for the frame log.
pub mod can_class {
    /// Device status broadcast — periodic self-announcement (liveness).
    pub const DEVICE_BROADCAST: u8 = 0x04;
    /// Bus-master heartbeat (0-byte frame from the master node).
    pub const BUS_POLL: u8 = 0x05;
    /// Property/count response from a device.
    pub const PROPERTY_INFO: u8 = 0x06;
    /// Property/count request to a device.
    pub const PROPERTY_REQ: u8 = 0x07;
    /// Monitoring value (real address) or Monitoring property answer
    /// (address bit 23).
    pub const MONITORING_DATA: u8 = 0x08;
    /// Monitoring group answer (real address) or Alarm property answer
    /// (bit 23).
    pub const SCHEMA_DATA: u8 = 0x09;
    /// Alarm group answer (real address) or History property answer (bit 23).
    pub const SCHEMA_DATA_ALARM: u8 = 0x0A;
    /// History group answer (real address) or Configuration value (bit 23).
    pub const SCHEMA_DATA_HISTORY: u8 = 0x0B;
    /// Configuration property answer (real address) or Configuration group
    /// answer (bit 23).
    pub const BTM3_META_DATA: u8 = 0x0C;
    /// Monitoring value "not available", which is also a write's answer
    /// (real address), or Monitoring property "not available" (bit 23).
    pub const WRITE_ACK: u8 = 0x10;
    /// Monitoring group "not available" (real address) or Alarm property
    /// "not available" (bit 23).
    pub const SCHEMA_DATA_NA: u8 = 0x11;
    /// Monitoring value read or write (real address) or Monitoring property
    /// request (bit 23).
    pub const MONITORING_REQ: u8 = 0x18;
    /// Monitoring group request (real address) or Alarm property request
    /// (bit 23).
    pub const SCHEMA_REQ: u8 = 0x19;
    /// Alarm group request (real address) or History property request
    /// (bit 23).
    pub const SCHEMA_REQ_ALARM: u8 = 0x1A;
    /// History group request (real address) or Configuration value read or
    /// write (bit 23).
    pub const SCHEMA_REQ_HISTORY: u8 = 0x1B;
    /// Configuration property request (real address) or Configuration group
    /// request (bit 23).
    pub const BTM3_META_REQ: u8 = 0x1C;
}

/// Tab byte used in monitoring requests. The field index is global; the tab byte
/// is always 0 (a nonzero tab returns a class-`0x10` "no value").
pub const TAB_DEFAULT: u8 = 0x00;

/// `[0x08, selector]` selectors (PROTOCOL.md §4.3): per tab, the number of
/// values and of groups, as MasterAdjust names them (`Prod_GeneralToStr`).
pub mod menu {
    /// `MonitoringValues`.
    pub const MONITORING_VALUES: u8 = 0x01;
    /// `MonitoringGroups`.
    pub const MONITORING_GROUPS: u8 = 0x02;
    /// `HistoryValues`.
    pub const HISTORY_VALUES: u8 = 0x03;
    /// `HistoryGroups`.
    pub const HISTORY_GROUPS: u8 = 0x04;
    /// `ConfigurationValues`.
    pub const CONFIGURATION_VALUES: u8 = 0x05;
    /// `ConfigurationGroups`. Changes with the access level.
    pub const CONFIGURATION_GROUPS: u8 = 0x06;
    /// `AlarmsValues`.
    pub const ALARM_VALUES: u8 = 0x07;
    /// `AlarmsGroups`.
    pub const ALARM_GROUPS: u8 = 0x08;
    /// `EventGeneratingGroup`: the Configuration group that holds the
    /// device's events.
    pub const EVENT_GROUP: u8 = 0x3E;
    /// The number of eventable outputs, which MasterAdjust calls
    /// `Eventables` (PROTOCOL.md §4.3). Named `TOTAL` from an early reading
    /// as a total group count; kept under that name for compatibility.
    #[deprecated(note = "0x3F is the eventable-output count; use `EVENTABLES`")]
    pub const TOTAL: u8 = 0x3F;
    /// The number of eventable outputs (`Eventables` in MasterAdjust); output
    /// `K` is then asked with [`EVENTABLE_0`]` + K` (PROTOCOL.md §4.3, §9a).
    pub const EVENTABLES: u8 = 0x3F;
    /// The selector of eventable output 0; output `K` is `EVENTABLE_0 + K`.
    pub const EVENTABLE_0: u8 = 0x40;
    /// The most eventable outputs MasterAdjust accepts from a device.
    pub const MAX_EVENTABLES: u8 = 32;
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
    /// Eventable flag (`byte[4]`: 1 = this field is a controllable output that
    /// an event on another device can drive). The device's eventable fields are
    /// the `Event N command` target space (see model/discovery).
    ///
    /// A device answers this **only for its eventable fields** — non-eventable
    /// fields stay silent — so discovery queries it *best-effort* and only on
    /// Monitoring fields (the only tab that carries eventable outputs). A
    /// blocking query on every field would pay a full timeout on the silent
    /// majority. (Note the same byte `0x0D` is also a device-level
    /// `[0x08, 0x0D]` selector for the *count* of eventable fields; that's a
    /// different query.)
    pub const EVENTABLE: u8 = 0x0D;
    /// Unit string id.
    pub const UNIT: u8 = 0x2C;
    /// Option string id (`[0x26, field, 0x00, opt_idx]`).
    pub const OPTION: u8 = 0x26;
}

/// Address bit 23, which half of a tab's message types use (see [`tab`]).
/// Wire-level only; not part of the public field-id space.
pub(crate) const BTM1_META_ADDR_FLAG: u32 = tab::HIGH;

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
    /// Event command: selects which of the target device's eventable outputs an
    /// event drives. The value is an index into the *target's* ordered
    /// eventable fields (the target is the sibling [`Self::DeviceList`] field).
    EventCommand,
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
        // Event command: an index into the target device's eventable outputs.
        0x0A => VisualizationType::EventCommand,
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
