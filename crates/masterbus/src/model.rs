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

/// A menu / access level (the `[0x08, selector]` partition of the global group
/// list — e.g. monitoring, configuration/installer, service).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Disk-cache key: `article + firmware` identifies the schema.
    pub fn cache_key(article: &str, firmware: &str) -> String {
        format!("{}-{}", article, firmware)
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
