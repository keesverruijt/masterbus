//! The curated MasterBus → Signal K field mapping.
//!
//! # Why this file exists
//!
//! The sidecar used to decide what a field meant by matching its *name*
//! against a per-class table. Three things on a real 28-device bus defeat that
//! (see issue #12):
//!
//! - Field names are installer-editable. A charger's `Output 1` is routinely
//!   renamed `Eng.batt`.
//! - The class prefix is not a schema key. Two charger articles both advertise
//!   as `CHG`, both present a group called `Output`, and the contents are
//!   unrelated.
//! - Even factory names vary by model. One battery family calls its terminal
//!   voltage `Battery`, another calls it `Voltage`, so a name table silently
//!   drops the core measurements of a supposedly supported class.
//!
//! So the mapping is keyed on what the firmware fixes rather than on what a
//! human typed: the device's **serial number**, and the **channel-aware field
//! id** within it. Serial is immutable per physical unit and survives every
//! rename. Field ids are stable per unit, which is stronger than stable per
//! article: an MLI Ultra acting as cluster master lays its groups out
//! differently from the same model acting as a member, and a per-serial file
//! never has to model that.
//!
//! # What is not in here
//!
//! No scale factors. A mapping says which Signal K path a field publishes to;
//! the arithmetic is derived from the field's unit and the unit that path's
//! leaf wants (see [`crate::units::conversion`]). A stored factor would be one
//! more thing to get wrong. The single exception is [`FieldMapping::invert`],
//! for the boolean case no unit can express: a charger reporting `Standby`
//! publishes to `enabled` negated.
//!
//! # Format
//!
//! ```json
//! {
//!   "version": 1,
//!   "devices": {
//!     "R516V1070": {
//!       "article": "26024000",
//!       "firmware": "2.65",
//!       "name": "MSU Inverter",
//!       "instance": "inverter",
//!       "fields": {
//!         "0x006": { "path": "electrical.inverters.inverter.dc.voltage" },
//!         "0x015": { "path": "electrical.chargers.inverter.enabled", "invert": true }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Presence is the toggle: a field that is not listed is not published. There
//! are no per-menu or per-group default flags, and so no precedence rules.

use std::collections::BTreeMap;
use std::path::Path;

use masterbus::FieldId;
use serde::{Deserialize, Serialize};

/// Current value of the file's `version` key.
pub const VERSION: u32 = 1;

/// A whole mapping file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    /// Format version; [`VERSION`] for files this build writes.
    pub version: u32,
    /// Per-device mappings, keyed by the device's serial number.
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceMapping>,
}

/// One device's mapping.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceMapping {
    /// Article (model) number. Informational, refreshed on discovery; the key
    /// a suggestion database is looked up by.
    #[serde(default)]
    pub article: String,
    /// Firmware version at the time the mapping was written. Informational;
    /// a change is worth warning about, because field ids may have moved.
    #[serde(default)]
    pub firmware: String,
    /// The device's name when the mapping was written. Purely to make the file
    /// readable — it is never matched on.
    #[serde(default)]
    pub name: String,
    /// The Signal K instance the paths below were built with. Recorded so a
    /// rename is one edit plus a rebuild, not a hand-edit of every path.
    #[serde(default)]
    pub instance: String,
    /// Field id (as `0x000`..`0x1FF` text) → what it publishes.
    #[serde(default)]
    pub fields: BTreeMap<String, FieldMapping>,
}

/// What one field publishes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldMapping {
    /// Full Signal K path, instance included.
    pub path: String,
    /// Publish the logical negation of a boolean field. The one transform no
    /// unit pair can express: a charger's `Standby` is `enabled` inverted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
}

/// Render a field id the way the whole toolchain does: `0x000`..`0x1FF`, three
/// hex digits of the channel-aware id.
pub fn field_key(id: FieldId) -> String {
    format!("0x{id:03X}")
}

/// Parse a field key written by [`field_key`]. Accepts the `0x` prefix in
/// either case, and a bare hex number, so a hand-edited file is forgiving.
pub fn parse_field_key(key: &str) -> Option<FieldId> {
    let t = key.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    FieldId::from_str_radix(t, 16).ok()
}

impl Mapping {
    /// An empty mapping stamped with the current version.
    pub fn new() -> Mapping {
        Mapping {
            version: VERSION,
            devices: BTreeMap::new(),
        }
    }

    /// Read a mapping file. A missing file is an empty mapping, not an error:
    /// that is the normal state before anyone has curated anything.
    pub fn load(path: &Path) -> std::io::Result<Mapping> {
        let raw = match std::fs::read(path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Mapping::new()),
            Err(e) => return Err(e),
        };
        serde_json::from_slice(&raw).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        })
    }

    /// Write the mapping, creating the parent directory if needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut body = serde_json::to_vec_pretty(self)?;
        body.push(b'\n');
        std::fs::write(path, body)
    }

    /// Look up one field's mapping by device serial and field id.
    pub fn field(&self, serial: &str, id: FieldId) -> Option<&FieldMapping> {
        self.devices.get(serial)?.fields.get(&field_key(id))
    }

    /// Total number of mapped fields across all devices.
    pub fn len(&self) -> usize {
        self.devices.values().map(|d| d.fields.len()).sum()
    }

    /// Whether anything at all is mapped.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Mapping {
        let mut m = Mapping::new();
        let mut d = DeviceMapping {
            article: "26024000".into(),
            firmware: "2.65".into(),
            name: "MSU Inverter".into(),
            instance: "inverter".into(),
            ..Default::default()
        };
        d.fields.insert(
            field_key(0x006),
            FieldMapping {
                path: "electrical.inverters.inverter.dc.voltage".into(),
                invert: false,
            },
        );
        d.fields.insert(
            field_key(0x015),
            FieldMapping {
                path: "electrical.chargers.inverter.enabled".into(),
                invert: true,
            },
        );
        m.devices.insert("R516V1070".into(), d);
        m
    }

    #[test]
    fn field_keys_match_the_toolchain_encoding() {
        assert_eq!(field_key(0x006), "0x006");
        assert_eq!(field_key(0x10C), "0x10C");
        assert_eq!(parse_field_key("0x10C"), Some(0x10C));
        assert_eq!(parse_field_key("10c"), Some(0x10C));
        assert_eq!(parse_field_key(" 0X006 "), Some(0x006));
        assert_eq!(parse_field_key("zz"), None);
    }

    #[test]
    fn round_trips_through_json() {
        let m = sample();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let back: Mapping = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn invert_is_omitted_when_false() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert_eq!(json.matches("\"invert\"").count(), 1, "only the true one");
        assert!(
            json.contains(
                r#""0x015":{"path":"electrical.chargers.inverter.enabled","invert":true}"#
            )
        );
    }

    #[test]
    fn lookup_is_by_serial_and_field_id() {
        let m = sample();
        assert_eq!(
            m.field("R516V1070", 0x006).map(|f| f.path.as_str()),
            Some("electrical.inverters.inverter.dc.voltage")
        );
        assert!(m.field("R516V1070", 0x999).is_none());
        assert!(m.field("someone-elses-unit", 0x006).is_none());
    }

    #[test]
    fn a_missing_file_is_an_empty_mapping_not_an_error() {
        let p = std::env::temp_dir().join("masterbus-no-such-mapping-file.json");
        let _ = std::fs::remove_file(&p);
        let m = Mapping::load(&p).expect("missing file must not be an error");
        assert!(m.is_empty());
        assert_eq!(m.version, VERSION);
    }

    #[test]
    fn saves_and_loads_from_disk() {
        let dir = std::env::temp_dir().join(format!("masterbus-map-{}", std::process::id()));
        let p = dir.join("mapping.json");
        let _ = std::fs::remove_dir_all(&dir);
        sample().save(&p).expect("save creates the directory");
        assert_eq!(Mapping::load(&p).unwrap(), sample());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_json_reports_the_path() {
        let dir = std::env::temp_dir().join(format!("masterbus-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("mapping.json");
        std::fs::write(&p, b"{ not json").unwrap();
        let e = Mapping::load(&p).unwrap_err();
        assert!(e.to_string().contains("mapping.json"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_fields_across_devices() {
        assert_eq!(sample().len(), 2);
        assert!(!sample().is_empty());
        assert!(Mapping::new().is_empty());
    }
}
