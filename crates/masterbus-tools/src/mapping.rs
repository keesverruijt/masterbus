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
//! the arithmetic and the unit metadata are derived from the field's own unit
//! (see [`crate::units::to_si`]). A stored factor would be one more thing to
//! get wrong. The exceptions are the two things no unit can express:
//! [`FieldMapping::invert`], for a charger reporting `Standby` that publishes
//! to `enabled` negated, and [`FieldMapping::truth`], for an enum such as
//! `Standby` / `On` / `Alarm` published to a boolean leaf. And one thing that
//! is not a value at all: [`FieldMapping::notify`], the labels of an enum that
//! should raise a Signal K notification, because `Alarm` is something a
//! server should act on rather than a string on a dashboard.
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
//!         "0x015": { "path": "electrical.chargers.inverter.enabled", "invert": true },
//!         "0x010": { "path": "electrical.inverters.inverter.inverterMode",
//!                    "notify": { "Alarm": "alarm" } }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Presence is the toggle: a field that is not listed is not published. There
//! are no per-menu or per-group default flags, and so no precedence rules.
//!
//! A field may additionally be marked `"put": true`, which is how a Signal K
//! PUT on its path reaches the bus (see [`FieldMapping::put`]).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use masterbus::{DeviceIdentity, FieldId};
use serde::{Deserialize, Serialize};

use crate::signalk;

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
    /// For an enum published to a boolean leaf: which label means what.
    /// `{"Standby": false, "On": true}`. Empty for anything else. The editor
    /// fills it in from the conventional meanings where it can, and asks for
    /// the rest (`Alarm`?); a label missing from the table publishes nothing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub truth: BTreeMap<String, bool>,
    /// For an enum: which labels raise a Signal K notification, and how
    /// loudly. `{"Alarm": "alarm", "Overload": "warn"}`. While the field's
    /// label is listed, `notifications.<path>` carries that state; when it
    /// leaves the list, `normal`. Empty for anything else.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub notify: BTreeMap<String, NotifyState>,
    /// Also accept writes: the Signal K plugin registers a PUT handler on
    /// the path, and a PUT of an SI value is converted back and written to
    /// the field. Only meaningful on a writable field; the field still
    /// publishes its value as usual, so Signal K shows the state it set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub put: bool,
}

/// A Signal K notification state worth raising. `normal` is not listed: it
/// is what a label that is not in the table means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyState {
    /// Something to notice; no sound.
    Alert,
    /// Something to look at soon; no sound.
    Warn,
    /// Something wrong now; visual and sound.
    Alarm,
    /// Something dangerous now; visual and sound.
    Emergency,
}

impl NotifyState {
    /// The state as Signal K spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            NotifyState::Alert => "alert",
            NotifyState::Warn => "warn",
            NotifyState::Alarm => "alarm",
            NotifyState::Emergency => "emergency",
        }
    }

    /// How the notification should be brought to attention, per the spec's
    /// `method` field.
    pub fn methods(self) -> &'static [&'static str] {
        match self {
            NotifyState::Alert | NotifyState::Warn => &["visual"],
            NotifyState::Alarm | NotifyState::Emergency => &["visual", "sound"],
        }
    }

    /// Every state, quietest first; what the editor cycles through.
    pub const ALL: [NotifyState; 4] = [
        NotifyState::Alert,
        NotifyState::Warn,
        NotifyState::Alarm,
        NotifyState::Emergency,
    ];
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

/// A device one device's mapping can be copied onto: the same article, a
/// different serial.
#[derive(Debug, Clone)]
pub struct CopyTarget {
    /// Signal K instance proposed for it, used when it has no entry yet.
    pub instance: String,
    /// The field ids it actually has, so nothing is written blind.
    pub have: HashSet<FieldId>,
    /// Its identity, recorded into the new entry.
    pub ident: DeviceIdentity,
}

/// Copy one device's field mappings onto every target, substituting each
/// target's own Signal K instance into the paths. Returns (copied, skipped).
///
/// This is the TUI's `a` key and the API's `apply-article`, so it lives here
/// rather than in either.
///
/// Fields the target does not have are skipped rather than written blind. That
/// is what keeps a cluster master's extra fields off a plain member of the same
/// article, which is the case the bus in #6 actually contains.
///
/// The instance substituted is the one each *path* uses, not the one recorded
/// for the device. A cluster master publishes its aggregate under one node
/// and its own cells under another, so its entries do not share an instance;
/// substituting the device's single recorded one copied nothing at all when
/// pressed on the master (#12).
pub fn copy_to_targets(
    map: &mut Mapping,
    src: &DeviceMapping,
    targets: &[CopyTarget],
) -> (usize, usize) {
    let mut copied = 0usize;
    let mut skipped = 0usize;
    for t in targets {
        let entry = map.devices.entry(t.ident.serial.clone()).or_default();
        entry.article = t.ident.article.clone();
        entry.firmware = t.ident.firmware.clone();
        entry.name = t.ident.name.clone();
        if entry.instance.is_empty() {
            entry.instance = t.instance.clone();
        }
        let target_instance = entry.instance.clone();
        for (key, fm) in &src.fields {
            match parse_field_key(key) {
                Some(id) if t.have.contains(&id) => {
                    let from =
                        signalk::instance_of(&fm.path).unwrap_or_else(|| src.instance.clone());
                    let path = retarget(&fm.path, &from, &target_instance);
                    // Nothing was substituted, so this target would publish to
                    // the source's own node. Two devices writing one path is
                    // never what "apply to this article" meant.
                    if path == fm.path {
                        skipped += 1;
                        continue;
                    }
                    entry
                        .fields
                        .insert(key.to_string(), FieldMapping { path, ..fm.clone() });
                    copied += 1;
                }
                _ => skipped += 1,
            }
        }
    }
    (copied, skipped)
}

/// Swap one instance segment for another inside a Signal K path.
///
/// Only whole segments are replaced, so an instance that happens to be a
/// substring of a leaf (`house` in `household`) is left alone. An empty source
/// instance means there is nothing to substitute and the path is copied as-is.
pub fn retarget(path: &str, from: &str, to: &str) -> String {
    if from.is_empty() || from == to {
        return path.to_string();
    }
    path.split('.')
        .map(|seg| if seg == from { to } else { seg })
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed;

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
                ..Default::default()
            },
        );
        d.fields.insert(
            field_key(0x015),
            FieldMapping {
                path: "electrical.chargers.inverter.enabled".into(),
                invert: true,
                ..Default::default()
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

    #[test]
    fn notify_states_round_trip_in_lowercase_and_are_omitted_when_empty() {
        let mut m = sample();
        let d = m.devices.get_mut("R516V1070").unwrap();
        d.fields.insert(
            field_key(0x010),
            FieldMapping {
                path: "electrical.inverters.inverter.inverterMode".into(),
                notify: [
                    ("Alarm", NotifyState::Alarm),
                    ("Overload", NotifyState::Warn),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json.matches("\"notify\"").count(), 1);
        assert!(json.contains(r#""notify":{"Alarm":"alarm","Overload":"warn"}"#));
        let back: Mapping = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
        assert_eq!(NotifyState::Alarm.methods(), &["visual", "sound"]);
        assert_eq!(NotifyState::Warn.methods(), &["visual"]);
    }

    #[test]
    fn a_truth_table_round_trips_and_is_omitted_when_empty() {
        let mut m = sample();
        let d = m.devices.get_mut("R516V1070").unwrap();
        d.fields.insert(
            field_key(0x010),
            FieldMapping {
                path: "electrical.switches.inverter.state".into(),
                truth: [("Standby", false), ("On", true)]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json.matches("\"truth\"").count(), 1, "only the enum entry");
        assert!(json.contains(r#""truth":{"On":true,"Standby":false}"#));
        let back: Mapping = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    /// `put` is a flag like `invert`: absent when false, so a file from a
    /// build that predates it reads back unchanged, and a file this build
    /// writes stays readable by one that ignores unknown keys.
    #[test]
    fn put_is_omitted_when_false_and_round_trips_when_set() {
        let mut m = sample();
        let d = m.devices.get_mut("R516V1070").unwrap();
        d.fields.insert(
            field_key(0x013),
            FieldMapping {
                path: "electrical.inverters.inverter.enabled".into(),
                put: true,
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json.matches("\"put\"").count(), 1);
        assert!(
            json.contains(r#""0x013":{"path":"electrical.inverters.inverter.enabled","put":true}"#)
        );
        let back: Mapping = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
        assert!(!back.devices["R516V1070"].fields[&field_key(0x006)].put);
    }

    fn ident(serial: &str, name: &str) -> DeviceIdentity {
        DeviceIdentity {
            article: "66026000".into(),
            serial: serial.into(),
            revision: "A".into(),
            name: name.into(),
            firmware: "2.14".into(),
        }
    }

    fn src_mapping() -> DeviceMapping {
        let mut d = DeviceMapping {
            article: "66026000".into(),
            firmware: "2.14".into(),
            name: "BAT 24V Service".into(),
            instance: "24v-service".into(),
            ..Default::default()
        };
        for (id, leaf) in [
            (0x001u16, "voltage"),
            (0x002, "current"),
            (0x071, "voltage"),
        ] {
            d.fields.insert(
                field_key(id),
                FieldMapping {
                    path: format!("electrical.batteries.24v-service.{leaf}"),
                    ..Default::default()
                },
            );
        }
        d
    }

    fn target(serial: &str, name: &str, have: &[FieldId]) -> CopyTarget {
        CopyTarget {
            instance: seed::instance_of(name, 0x1000),
            have: have.iter().copied().collect(),
            ident: ident(serial, name),
        }
    }

    #[test]
    fn copying_rewrites_the_instance_segment_per_target() {
        let mut map = Mapping::new();
        let t = target("MLI-2", "BAT 24V Service2", &[0x001, 0x002, 0x071]);
        let (copied, skipped) = copy_to_targets(&mut map, &src_mapping(), &[t]);
        assert_eq!((copied, skipped), (3, 0));
        let d = &map.devices["MLI-2"];
        assert_eq!(d.instance, "24v-service2");
        assert_eq!(
            d.fields[&field_key(0x001)].path,
            "electrical.batteries.24v-service2.voltage"
        );
    }

    /// The cluster case from the bus in #6: two units share an article, but the
    /// master has fields the members do not. Copying must not invent them.
    #[test]
    fn fields_the_target_lacks_are_skipped_not_invented() {
        let mut map = Mapping::new();
        let member = target("MLI-2", "BAT 24V Service2", &[0x001, 0x002]);
        let (copied, skipped) = copy_to_targets(&mut map, &src_mapping(), &[member]);
        assert_eq!((copied, skipped), (2, 1));
        let d = &map.devices["MLI-2"];
        assert!(!d.fields.contains_key(&field_key(0x071)));
    }

    #[test]
    fn copying_records_the_target_identity_not_the_sources() {
        let mut map = Mapping::new();
        let mut t = target("MLI-2", "BAT 24V Service2", &[0x001]);
        t.ident.firmware = "2.15".into();
        copy_to_targets(&mut map, &src_mapping(), &[t]);
        let d = &map.devices["MLI-2"];
        assert_eq!(d.name, "BAT 24V Service2");
        assert_eq!(d.firmware, "2.15");
    }

    /// An instance the user already chose is authoritative; copying must not
    /// silently rename a device's Signal K node underneath them.
    #[test]
    fn an_existing_instance_on_the_target_is_kept() {
        let mut map = Mapping::new();
        map.devices.insert(
            "MLI-2".into(),
            DeviceMapping {
                instance: "port-bank".into(),
                ..Default::default()
            },
        );
        let t = target("MLI-2", "BAT 24V Service2", &[0x001]);
        copy_to_targets(&mut map, &src_mapping(), &[t]);
        let d = &map.devices["MLI-2"];
        assert_eq!(d.instance, "port-bank");
        assert_eq!(
            d.fields[&field_key(0x001)].path,
            "electrical.batteries.port-bank.voltage"
        );
    }

    /// From real use on a live boat: an `INT Nav Chg` was mapped by hand onto
    /// `electrical.chargers.nav-battery`, because that is what it charges,
    /// while the device's recorded instance was still `nav-chg`. Substituting
    /// on the recorded instance copied nothing; substituting on the instance
    /// the path itself uses copies it correctly, so a stale record no longer
    /// matters.
    #[test]
    fn a_stale_recorded_instance_does_not_block_a_copy() {
        let mut map = Mapping::new();
        let mut src = DeviceMapping {
            article: "77030450".into(),
            instance: "nav-chg".into(),
            ..Default::default()
        };
        src.fields.insert(
            field_key(0x028),
            FieldMapping {
                path: "electrical.chargers.nav-battery.voltage".into(),
                ..Default::default()
            },
        );
        let t = target("X922S0096", "INT 24V DC/DC", &[0x028]);
        let (copied, skipped) = copy_to_targets(&mut map, &src, &[t]);
        assert_eq!((copied, skipped), (1, 0));
        assert_eq!(
            map.devices["X922S0096"].fields[&field_key(0x028)].path,
            "electrical.chargers.24v-dc-dc.voltage"
        );
    }

    #[test]
    fn retarget_replaces_whole_segments_only() {
        assert_eq!(
            retarget("electrical.batteries.house.voltage", "house", "port"),
            "electrical.batteries.port.voltage"
        );
        // A leaf that merely contains the instance as a substring is untouched.
        assert_eq!(
            retarget("electrical.batteries.house.household", "house", "port"),
            "electrical.batteries.port.household"
        );
        // Nothing to substitute.
        assert_eq!(retarget("a.b.c", "", "port"), "a.b.c");
        assert_eq!(retarget("a.b.c", "b", "b"), "a.b.c");
    }

    /// The cluster-master case from the field report on #12: the master maps
    /// its aggregate under one instance and its own cell under another. A copy
    /// from the master must substitute on what each path uses, or nothing
    /// matches the device's single recorded instance and every field skips.
    #[test]
    fn a_copy_substitutes_on_each_paths_own_instance() {
        let mut map = Mapping::new();
        let mut master = DeviceMapping {
            article: "66026000".into(),
            // Recorded from the last path saved: the cell's.
            instance: "li-ion-1".into(),
            ..Default::default()
        };
        // Cluster group: aggregate node.
        master.fields.insert(
            field_key(0x001),
            FieldMapping {
                path: "electrical.batteries.li-ion.voltage".into(),
                ..Default::default()
            },
        );
        // Own battery group: the cell's node.
        master.fields.insert(
            field_key(0x071),
            FieldMapping {
                path: "electrical.batteries.li-ion-1.voltage".into(),
                ..Default::default()
            },
        );
        // A plain member only has the 0x000-range group.
        let member = target("MLI-2", "BAT li-ion 2", &[0x001]);
        let (copied, skipped) = copy_to_targets(&mut map, &master, &[member]);
        assert_eq!((copied, skipped), (1, 1));
        assert_eq!(
            map.devices["MLI-2"].fields[&field_key(0x001)].path,
            "electrical.batteries.li-ion-2.voltage"
        );
    }

    /// Everything an entry says about *how* to publish travels with the copy:
    /// the truth table, the notification table, and the put flag.
    #[test]
    fn a_copy_carries_the_truth_notify_and_put_settings() {
        let mut map = Mapping::new();
        let mut src = DeviceMapping {
            article: "77010100".into(),
            instance: "out-1".into(),
            ..Default::default()
        };
        src.fields.insert(
            field_key(0x001),
            FieldMapping {
                path: "electrical.switches.out-1.state".into(),
                truth: [
                    ("Standby".to_string(), false),
                    ("Activated".to_string(), true),
                ]
                .into(),
                notify: [("Alarm".to_string(), NotifyState::Alarm)].into(),
                put: true,
                ..Default::default()
            },
        );
        let t = target("MCO-2", "INT Out 2", &[0x001]);
        copy_to_targets(&mut map, &src, &[t]);
        let f = &map.devices["MCO-2"].fields[&field_key(0x001)];
        assert_eq!(f.path, "electrical.switches.out-2.state");
        assert_eq!(f.truth.len(), 2);
        assert_eq!(f.notify.len(), 1);
        assert!(f.put);
    }
}
