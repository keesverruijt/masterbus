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

/// A menu / tab on the device. The first three live in the global gid space
/// addressed via class `0x19` schema requests and `[0x08, selector]` counts.
/// `Alarm` and `History` use parallel schema channels (`0x1A` / `0x1B`) with
/// their own gid namespaces (each starting at 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Menu {
    /// User-visible monitoring (selector 0x02, schema channel 0x19).
    Monitoring,
    /// Configuration / installer (selector 0x03, schema channel 0x19).
    Configuration,
    /// Service (selector 0x04, schema channel 0x19).
    Service,
    /// Alarms tab (schema channel 0x1A, separate gid namespace).
    Alarm,
    /// History / Service-events tab (schema channel 0x1B, separate gid namespace).
    History,
    /// Any other selector value.
    Other(u8),
}

impl Menu {
    /// The wire selector byte for this menu's group-count query (`[0x08, sel]`).
    /// [`Menu::Alarm`] and [`Menu::History`] don't fit this query pattern —
    /// their group counts are discovered by probe-and-stop on their own
    /// schema channel; callers should branch on the menu before relying on
    /// this byte.
    pub fn selector(self) -> u8 {
        match self {
            Menu::Monitoring => 0x02,
            Menu::Configuration => 0x03,
            Menu::Service => 0x04,
            // Hypothesised: `[0x08, 0x08]` is the Alarm-group count on devices
            // that have one — matches the 2 Alarm groups on the CombiMaster and
            // the 0 on the Magic-class Nav Chg. Treat as TBD until cross-device
            // confirmation; falls back to probe-and-stop in any case.
            Menu::Alarm => 0x08,
            // No known selector for History; rely on probe-and-stop.
            Menu::History => 0x00,
            Menu::Other(s) => s,
        }
    }

    /// CAN class of the schema-request channel that enumerates this menu's
    /// groups. Pairs with the matching response class
    /// (`SCHEMA_DATA*` = class - 0x10).
    pub fn schema_request_class(self) -> u8 {
        use crate::protocol::can_class;
        match self {
            Menu::Monitoring | Menu::Configuration | Menu::Service | Menu::Other(_) => {
                can_class::SCHEMA_REQ
            }
            Menu::Alarm => can_class::SCHEMA_REQ_ALARM,
            Menu::History => can_class::SCHEMA_REQ_HISTORY,
        }
    }

    /// Map a wire selector byte to a [`Menu`] (only well-defined for the
    /// global-gid menus Monitoring / Configuration / Service).
    pub fn from_selector(s: u8) -> Menu {
        match s {
            0x02 => Menu::Monitoring,
            0x03 => Menu::Configuration,
            0x04 => Menu::Service,
            other => Menu::Other(other),
        }
    }
}

/// A device's 24-bit address on the bus — the unique id by which the rest of
/// the API addresses it. Carried in bits 23..0 of every CAN frame's id; the
/// top 8 bits of the wire `u32` are never set. Aliased rather than newtyped
/// so collection keys and literal addresses (`0x188EA2`) stay ergonomic.
pub type DeviceId = u32;

/// Which on-wire metadata channel a field lives on. The two channels share
/// the byte-sized wire field-index space but expose **different fields per
/// index** (different names, different live values). See PROTOCOL.md §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Channel {
    /// Btm1 — the legacy MasterBus protocol (named after the BTM battery
    /// monitor family). Per-field metadata `0x18 → 0x08` on `addr | 0x800000`;
    /// values + writes on the real address.
    Btm1,
    /// Btm3 — MasterBus revision 3 (BTM-III). Per-field metadata
    /// `0x1C → 0x0C` on the real address; headerless values + writes on
    /// `addr | 0x800000` via classes `0x1B`/`0x0B`.
    Btm3,
}

/// Channel-aware field id. Bits `0..8` are the on-wire 8-bit field index
/// within the channel; bit 8 selects the channel (`0` = Btm1, `1` = Btm3);
/// bits `9..16` are reserved for future channels and must currently be `0`.
///
/// `FieldId` is a `u16` typedef rather than a newtype to keep collection
/// keys (`HashMap<FieldId, _>`) and serde encodings boring.
pub type FieldId = u16;

/// Helpers for composing and inspecting [`FieldId`]s.
pub mod field_id {
    use super::{Channel, FieldId};

    /// Bit that distinguishes the channel. Clear = Btm1; set = Btm3.
    pub const CHANNEL_BIT: FieldId = 0x0100;

    /// Compose a `FieldId` for a Btm1 wire index.
    pub const fn btm1(wire: u8) -> FieldId {
        wire as FieldId
    }

    /// Compose a `FieldId` for a Btm3 wire index.
    pub const fn btm3(wire: u8) -> FieldId {
        (wire as FieldId) | CHANNEL_BIT
    }

    /// The channel this field id refers to.
    pub const fn channel(id: FieldId) -> Channel {
        if id & CHANNEL_BIT != 0 {
            Channel::Btm3
        } else {
            Channel::Btm1
        }
    }

    /// The 8-bit wire field index within its channel (the byte that goes
    /// out in the request payload). Bits above 8 are masked off.
    pub const fn wire_index(id: FieldId) -> u8 {
        (id & 0xFF) as u8
    }
}

/// One field within a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Channel-aware field id (channel in bit 8, wire index in bits 0..8).
    /// See [`field_id`] for accessors.
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
