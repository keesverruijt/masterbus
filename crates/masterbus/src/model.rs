//! Device schema model — the static, per-firmware description of a device that
//! discovery produces and the optional disk cache persists.

use serde::{Deserialize, Serialize};

use crate::protocol::VisualizationType;

/// Operational status of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    /// Not responding / not present.
    Offline,
    /// Low-power sleep.
    Sleeping,
    /// On / normal.
    On,
    /// On with a warning.
    OnWarning,
    /// Off due to a fault.
    OffFault,
    /// Off due to an error.
    OffError,
    /// Firmware updating.
    Updating,
    /// Unknown.
    Unknown,
}

/// Per-device access level (the login state of opcode `0x08 0x19`).
/// Higher levels unlock additional writable fields. See PROTOCOL.md §4.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccessLevel {
    /// Default / logged-out state. No code required (logout target).
    EndUser = 0,
    /// Installer.
    Installer = 1,
    /// Distributor.
    Distributor = 2,
    /// MV Service.
    MvService = 3,
}

impl AccessLevel {
    /// The level byte transmitted in (and received on) the wire.
    pub fn level_byte(self) -> u8 {
        self as u8
    }

    /// Map a level byte (from a response or a poll reply) to an [`AccessLevel`].
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(AccessLevel::EndUser),
            1 => Some(AccessLevel::Installer),
            2 => Some(AccessLevel::Distributor),
            3 => Some(AccessLevel::MvService),
            _ => None,
        }
    }
}

/// One of a device's four tabs (PROTOCOL.md §2.1). Every device has all
/// four; each has its own groups, field properties and values, on CAN classes
/// of its own (see [`crate::protocol::tab`]), and numbers its fields from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Menu {
    /// Live readings and controls.
    Monitoring,
    /// Alarm states.
    Alarm,
    /// Counters and logged values.
    History,
    /// Settings, including the device's events.
    Configuration,
}

impl Menu {
    /// The four tabs, in MasterAdjust's order.
    pub const ALL: [Menu; 4] = [
        Menu::Monitoring,
        Menu::Alarm,
        Menu::History,
        Menu::Configuration,
    ];

    /// The `[0x08, sel]` selector for this tab's number of groups
    /// (PROTOCOL.md §4.3).
    pub fn groups_selector(self) -> u8 {
        use crate::protocol::menu;
        match self {
            Menu::Monitoring => menu::MONITORING_GROUPS,
            Menu::Alarm => menu::ALARM_GROUPS,
            Menu::History => menu::HISTORY_GROUPS,
            Menu::Configuration => menu::CONFIGURATION_GROUPS,
        }
    }

    /// The `[0x08, sel]` selector for this tab's number of values.
    pub fn values_selector(self) -> u8 {
        use crate::protocol::menu;
        match self {
            Menu::Monitoring => menu::MONITORING_VALUES,
            Menu::Alarm => menu::ALARM_VALUES,
            Menu::History => menu::HISTORY_VALUES,
            Menu::Configuration => menu::CONFIGURATION_VALUES,
        }
    }

    /// The tab's number in a [`FieldId`] (bits 8 and 9). Monitoring and
    /// Configuration keep the numbers of the two "channels" earlier
    /// versions had, so stored field ids stay valid.
    const fn code(self) -> u16 {
        match self {
            Menu::Monitoring => 0,
            Menu::Configuration => 1,
            Menu::Alarm => 2,
            Menu::History => 3,
        }
    }

    /// Short lower-case name, as used in waiter keys and file names.
    pub fn tag(self) -> &'static str {
        match self {
            Menu::Monitoring => "monitoring",
            Menu::Alarm => "alarm",
            Menu::History => "history",
            Menu::Configuration => "configuration",
        }
    }
}

/// A device's 24-bit address on the bus — the unique id by which the rest of
/// the API addresses it. Carried in bits 23..0 of every CAN frame's id; the
/// top 8 bits of the wire `u32` are never set. Aliased rather than newtyped
/// so collection keys and literal addresses (`0x188EA2`) stay ergonomic.
pub type DeviceId = u32;

/// Tab-aware field id. Bits `0..8` are the field's index within its tab,
/// bits `8..10` the tab ([`Menu`]): `0x0nn` Monitoring, `0x1nn`
/// Configuration, `0x2nn` Alarm, `0x3nn` History. A field index is sent as
/// 16 bits, but every device seen so far numbers its fields below 256.
///
/// `FieldId` is a `u16` typedef rather than a newtype to keep collection
/// keys (`HashMap<FieldId, _>`) and serde encodings boring.
pub type FieldId = u16;

/// Helpers for composing and inspecting [`FieldId`]s.
pub mod field_id {
    use super::{FieldId, Menu};

    /// The field id of index `index` on `menu`'s tab.
    pub const fn new(menu: Menu, index: u8) -> FieldId {
        (menu.code() << 8) | index as FieldId
    }

    /// A Monitoring field.
    pub const fn monitoring(index: u8) -> FieldId {
        new(Menu::Monitoring, index)
    }

    /// A Configuration field.
    pub const fn configuration(index: u8) -> FieldId {
        new(Menu::Configuration, index)
    }

    /// The tab this field id is on.
    pub const fn menu(id: FieldId) -> Menu {
        match (id >> 8) & 0x3 {
            0 => Menu::Monitoring,
            1 => Menu::Configuration,
            2 => Menu::Alarm,
            _ => Menu::History,
        }
    }

    /// The field's index within its tab, as sent on the wire.
    pub const fn index(id: FieldId) -> u8 {
        (id & 0xFF) as u8
    }
}

/// One field within a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Tab-aware field id. See [`field_id`] for accessors.
    pub index: FieldId,
    /// Field name.
    pub name: String,
    /// Unit (may be empty).
    pub unit: String,
    /// Presentation / edit widget.
    pub viz_type: VisualizationType,
    /// Whether the field is currently writable (meta op `0x0B`); gated fields
    /// read `false` until an access-level login is performed.
    pub writeable: bool,
    /// Whether the field is an **eventable output** (meta op `0x0D`): a
    /// controllable parameter that an event on another device can drive. A
    /// device's eventable fields, in index order, are the target space an
    /// `Event N command` selects into.
    #[serde(default)]
    pub eventable: bool,
    /// Minimum value (numeric fields).
    pub min: f64,
    /// Maximum value, or option count for lists.
    pub max: f64,
    /// Step (numeric fields).
    pub step: f64,
    /// Option labels (list/enum fields).
    pub options: Vec<String>,
}

/// A group of fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupInfo {
    /// Global group id.
    pub id: i32,
    /// Group name.
    pub name: String,
    /// The menu/access-level this group belongs to.
    pub menu: Menu,
    /// Fields in display order.
    pub fields: Vec<FieldInfo>,
}

impl GroupInfo {
    /// Find a field by its channel-aware id.
    pub fn field(&self, id: FieldId) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| f.index == id)
    }
}

/// A device's identity — the cheap half of discovery (no group/field/metadata
/// enumeration). Enough to label a device and key its schema cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// Article number.
    pub article: String,
    /// Serial number.
    pub serial: String,
    /// Revision code.
    pub revision: String,
    /// Human-readable device name.
    pub name: String,
    /// Firmware version string.
    pub firmware: String,
}

/// The discovered, cacheable schema of a device (static per firmware).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceSchema {
    /// Article number.
    pub article: String,
    /// Serial number.
    pub serial: String,
    /// Revision code.
    pub revision: String,
    /// Human-readable device name.
    pub name: String,
    /// Firmware version string.
    pub firmware: String,
    /// All discovered groups (across menus).
    pub groups: Vec<GroupInfo>,
}

impl DeviceSchema {
    /// Build a full schema from a fetched identity and enumerated groups.
    pub fn from_identity(identity: DeviceIdentity, groups: Vec<GroupInfo>) -> DeviceSchema {
        DeviceSchema {
            article: identity.article,
            serial: identity.serial,
            revision: identity.revision,
            name: identity.name,
            firmware: identity.firmware,
            groups,
        }
    }

    /// The identity portion of this schema.
    pub fn identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            article: self.article.clone(),
            serial: self.serial.clone(),
            revision: self.revision.clone(),
            name: self.name.clone(),
            firmware: self.firmware.clone(),
        }
    }

    /// Find a field anywhere in the schema by its channel-aware id.
    pub fn field(&self, id: FieldId) -> Option<&FieldInfo> {
        self.groups.iter().find_map(|g| g.field(id))
    }

    /// Groups belonging to a given menu.
    pub fn menu_groups(&self, menu: Menu) -> impl Iterator<Item = &GroupInfo> {
        self.groups.iter().filter(move |g| g.menu == menu)
    }
}
