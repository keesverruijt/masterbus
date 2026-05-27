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
    /// `code = <redacted>`.
    Installer = 1,
    /// `code = <redacted>`.
    Distributor = 2,
    /// `code = <redacted>`.
    MvService = 3,
}

impl AccessLevel {
    /// The level byte transmitted in (and received on) the wire.
    pub fn level_byte(self) -> u8 {
        self as u8
    }

    /// The IEEE-754 f32 access code that authenticates this level.
    /// [`Self::EndUser`] has no code (it is reached via logout, not login).
    pub fn code(self) -> Option<f32> {
        match self {
            AccessLevel::EndUser => None,
            AccessLevel::Installer => Some(<redacted>),
            AccessLevel::Distributor => Some(<redacted>),
            AccessLevel::MvService => Some(<redacted>),
        }
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

/// A menu / access level (the `[0x08, selector]` partition of the global group
/// list — e.g. monitoring, configuration/installer, service).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Menu {
    /// User-visible monitoring (selector 0x02).
    Monitoring,
    /// Configuration / installer (selector 0x03).
    Configuration,
    /// Service (selector 0x04).
    Service,
    /// Any other selector value.
    Other(u8),
}

impl Menu {
    /// The wire selector byte for this menu.
    pub fn selector(self) -> u8 {
        match self {
            Menu::Monitoring => 0x02,
            Menu::Configuration => 0x03,
            Menu::Service => 0x04,
            Menu::Other(s) => s,
        }
    }

    /// Map a wire selector byte to a [`Menu`].
    pub fn from_selector(s: u8) -> Menu {
        match s {
            0x02 => Menu::Monitoring,
            0x03 => Menu::Configuration,
            0x04 => Menu::Service,
            other => Menu::Other(other),
        }
    }
}

/// One field within a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Global field index (used in monitoring/shadow requests).
    pub index: i32,
    /// Field name.
    pub name: String,
    /// Unit (may be empty).
    pub unit: String,
    /// Presentation / edit widget.
    pub viz_type: VisualizationType,
    /// Whether the field is currently writable (shadow op `0x0B`); gated fields
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
    /// Find a field by its index.
    pub fn field(&self, index: i32) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| f.index == index)
    }
}

/// A device's identity — the cheap half of discovery (no group/field/shadow
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

    /// Find a field anywhere in the schema by its global index.
    pub fn field(&self, index: i32) -> Option<&FieldInfo> {
        self.groups.iter().find_map(|g| g.field(index))
    }

    /// Groups belonging to a given menu.
    pub fn menu_groups(&self, menu: Menu) -> impl Iterator<Item = &GroupInfo> {
        self.groups.iter().filter(move |g| g.menu == menu)
    }
}
