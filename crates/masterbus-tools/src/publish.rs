//! What a mapping resolves to against the live bus.
//!
//! `masterbus-signalk` has two consumers of this: the delta stream, which
//! needs to know which fields to subscribe to and how to encode each one, and
//! the HTTP control API, which needs the same device records to answer the
//! Signal K plugin's questions and to validate a mapping before it is saved.
//! Both used to live inside the binary; they are here so the two cannot
//! drift apart.
//!
//! Everything in here is pure given the device records: reading the bus is
//! the caller's job ([`DeviceRec::discover`] and [`DeviceRec::merge_groups`]
//! are the two places a record is built from bus data).

use std::collections::{HashMap, HashSet};
use std::fmt;

use masterbus::{DeviceId, FieldId, GroupInfo, Menu};
use serde::Serialize;

use crate::mapping::{DeviceMapping, FieldMapping, Mapping, field_key, parse_field_key};
use crate::seed;
use crate::signalk::{self, Plan};

/// The menu whose fields are offered for mapping by default. Configuration
/// and Service carry settings rather than measurements; they are discovered
/// on demand when a mapping names one of their fields (a writable setting
/// exposed as a Signal K PUT target) or when the editor asks for them.
pub const MENU: Menu = Menu::Monitoring;

/// One discovered device, reduced to what mapping needs.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRec {
    /// Bus address.
    pub id: DeviceId,
    /// Serial number — the mapping file's key for this unit.
    pub serial: String,
    /// Article (model) number.
    pub article: String,
    /// Installer-assigned name. Displayed and recorded, never matched on.
    pub name: String,
    /// Firmware version.
    pub firmware: String,
    /// Proposed Signal K instance id, used when seeding.
    pub instance: String,
    /// Fields as the device reports them, across every menu discovered so far.
    pub fields: Vec<FieldRec>,
    /// The menus whose fields are in `fields`.
    pub menus: Vec<Menu>,
}

/// One field, reduced to what mapping needs.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldRec {
    /// Channel-aware field id.
    pub id: FieldId,
    /// Name, as the installer left it.
    pub name: String,
    /// Unit, as the device reports it (may be empty).
    pub unit: String,
    /// Labels, for an enum; empty otherwise.
    pub options: Vec<String>,
    /// Whether the device currently accepts writes to it.
    pub writable: bool,
    /// The menu it was discovered on.
    pub menu: Menu,
    /// The group it sits in, for display.
    pub group: String,
}

impl DeviceRec {
    /// A device's identity and its fields on [`MENU`], as mapping needs them.
    /// A device that does not identify itself in time gets empty identity
    /// strings, which [`resolve`] treats as "cannot be mapped".
    pub fn discover(dev: &masterbus::Device) -> DeviceRec {
        let identity = dev
            .identity()
            .unwrap_or_else(|_| masterbus::DeviceIdentity {
                article: String::new(),
                serial: String::new(),
                revision: String::new(),
                name: String::new(),
                firmware: String::new(),
            });
        let mut rec = DeviceRec {
            id: dev.id(),
            instance: seed::instance_of(&identity.name, dev.id()),
            serial: identity.serial,
            article: identity.article,
            name: identity.name,
            firmware: identity.firmware,
            fields: Vec::new(),
            menus: Vec::new(),
        };
        rec.merge_groups(MENU, dev.tab_info(MENU).unwrap_or_default());
        rec
    }

    /// Add the fields of one discovered menu. A field id already present is
    /// left alone: the first menu to report an id owns it.
    pub fn merge_groups(&mut self, menu: Menu, groups: Vec<GroupInfo>) {
        if !self.menus.contains(&menu) {
            self.menus.push(menu);
        }
        for group in groups {
            for field in group.fields {
                if self.fields.iter().any(|f| f.id == field.index) {
                    continue;
                }
                self.fields.push(FieldRec {
                    id: field.index,
                    name: field.name,
                    unit: field.unit,
                    options: field.options,
                    writable: field.writeable,
                    menu,
                    group: group.name.clone(),
                });
            }
        }
    }

    /// Look a field up by id.
    pub fn field(&self, id: FieldId) -> Option<&FieldRec> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// The field ids this device has, for [`crate::mapping::CopyTarget`].
    pub fn field_ids(&self) -> HashSet<FieldId> {
        self.fields.iter().map(|f| f.id).collect()
    }

    /// The identity as the mapping records it.
    pub fn identity(&self) -> masterbus::DeviceIdentity {
        masterbus::DeviceIdentity {
            article: self.article.clone(),
            serial: self.serial.clone(),
            revision: String::new(),
            name: self.name.clone(),
            firmware: self.firmware.clone(),
        }
    }
}

/// The name of a menu as the API spells it.
pub fn menu_name(menu: Menu) -> &'static str {
    match menu {
        Menu::Monitoring => "monitoring",
        Menu::Configuration => "configuration",
        Menu::Service => "service",
        Menu::Alarm => "alarm",
        Menu::History => "history",
        Menu::Other(_) => "other",
    }
}

/// A menu by its API name.
pub fn menu_by_name(name: &str) -> Option<Menu> {
    Some(match name {
        "monitoring" => Menu::Monitoring,
        "configuration" => Menu::Configuration,
        "service" => Menu::Service,
        _ => return None,
    })
}

/// Everything needed to turn one field's updates into a Signal K value.
#[derive(Debug, Clone)]
pub struct Emit {
    /// Target Signal K path.
    pub path: String,
    /// How to get there, derived from the field's unit and the mapping entry.
    pub plan: Plan,
    /// The device's name, for notification messages.
    pub device: String,
    /// Whether the plugin should accept PUTs on the path.
    pub put: bool,
}

/// How much a diagnostic matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The entry is skipped; nothing publishes for it.
    Error,
    /// The entry publishes, but is worth a look.
    Warning,
    /// Nothing wrong; said once so it is not a surprise.
    Info,
}

/// One thing [`resolve`] has to say about a mapping. Printed to stderr by
/// the daemon and returned by the API, so the plugin's editor can show the
/// same words next to the entry they are about.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    /// How much it matters.
    pub severity: Severity,
    /// The device's serial, as the mapping keys it.
    pub serial: String,
    /// The device's name, if it is on the bus.
    pub device: String,
    /// The field key (`0x006`), when the diagnostic is about one field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The mapped path, when the diagnostic is about one field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What to do about it.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.device.is_empty() {
            write!(f, "{}", self.serial)?;
        } else {
            write!(f, "{} ({})", self.device, self.serial)?;
        }
        if let Some(k) = &self.field {
            write!(f, " {k}")?;
        }
        if let Some(p) = &self.path {
            write!(f, " → {p}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// What a mapping resolves to.
#[derive(Debug, Default)]
pub struct Resolved {
    /// What to subscribe to, and how to encode each update.
    pub emit: HashMap<(DeviceId, FieldId), Emit>,
    /// Everything that could not be honoured, or is worth a look.
    pub diagnostics: Vec<Diagnostic>,
}

impl Resolved {
    /// The number of diagnostics at a severity.
    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    }
}

/// Resolve the mapping against the live bus: which (device, field) pairs to
/// subscribe to, and how to encode each one. Entries that cannot be honoured
/// are reported rather than failing silently.
pub fn resolve(devices: &[DeviceRec], mapping: &Mapping) -> Resolved {
    let mut out = Resolved::default();
    let by_serial: HashMap<&str, &DeviceRec> = devices
        .iter()
        .filter(|d| !d.serial.is_empty())
        .map(|d| (d.serial.as_str(), d))
        .collect();

    for (serial, dm) in &mapping.devices {
        let diag = |severity,
                    device: &str,
                    field: Option<String>,
                    path: Option<String>,
                    message: String| Diagnostic {
            severity,
            serial: serial.clone(),
            device: device.to_string(),
            field,
            path,
            message,
        };
        let Some(dev) = by_serial.get(serial.as_str()) else {
            out.diagnostics.push(diag(
                Severity::Info,
                &dm.name,
                None,
                None,
                "not on the bus (yet — it is picked up if it announces itself later)".into(),
            ));
            continue;
        };
        if !dm.firmware.is_empty() && dm.firmware != dev.firmware {
            out.diagnostics.push(diag(
                Severity::Warning,
                &dev.name,
                None,
                None,
                format!(
                    "running firmware {} but its mapping was written for {}; field ids may \
                     have moved — check it",
                    dev.firmware, dm.firmware
                ),
            ));
        }
        for (key, fm) in &dm.fields {
            let Some(id) = parse_field_key(key) else {
                out.diagnostics.push(diag(
                    Severity::Error,
                    &dev.name,
                    Some(key.clone()),
                    Some(fm.path.clone()),
                    "is not a field id".into(),
                ));
                continue;
            };
            let Some(f) = dev.field(id) else {
                out.diagnostics.push(diag(
                    Severity::Error,
                    &dev.name,
                    Some(key.clone()),
                    Some(fm.path.clone()),
                    "the device has no such field".into(),
                ));
                continue;
            };
            // The unit and conversion follow from the field's own unit; the
            // leaf only cross-checks. What cannot work is skipped here, with
            // the reason, rather than published as a wrong number.
            let plan = match signalk::plan(&fm.path, &f.unit, &f.options, fm) {
                Ok(p) => p,
                Err(e) => {
                    out.diagnostics.push(diag(
                        Severity::Error,
                        &dev.name,
                        Some(key.clone()),
                        Some(fm.path.clone()),
                        format!("{e}; skipped"),
                    ));
                    continue;
                }
            };
            if let Some(w) = &plan.warning {
                out.diagnostics.push(diag(
                    Severity::Warning,
                    &dev.name,
                    Some(key.clone()),
                    Some(fm.path.clone()),
                    w.clone(),
                ));
            }
            if fm.put && !f.writable {
                out.diagnostics.push(diag(
                    Severity::Warning,
                    &dev.name,
                    Some(key.clone()),
                    Some(fm.path.clone()),
                    "accepts PUTs but the field is read-only at the current access level; \
                     a write needs an installer login"
                        .into(),
                ));
            }
            out.emit.insert(
                (dev.id, id),
                Emit {
                    path: fm.path.clone(),
                    plan,
                    device: dev.name.clone(),
                    put: fm.put,
                },
            );
        }
    }
    out
}

/// The field ids a mapping names on a device that the device record does not
/// have yet. Non-empty means a menu beyond [`MENU`] should be discovered
/// before resolving, because a writable setting lives on Configuration.
pub fn unknown_fields(dev: &DeviceRec, mapping: &Mapping) -> Vec<FieldId> {
    let Some(dm) = mapping.devices.get(&dev.serial) else {
        return Vec::new();
    };
    dm.fields
        .keys()
        .filter_map(|k| parse_field_key(k))
        .filter(|id| dev.field(*id).is_none())
        .collect()
}

/// Build a mapping from the bundled per-model database and the per-class
/// name heuristics — the migration path for an install that has no curated
/// file yet, and the starting point a human edits.
pub fn seed_mapping(devices: &[DeviceRec]) -> Mapping {
    let mut m = Mapping::new();
    for d in devices {
        if d.serial.is_empty() {
            continue;
        }
        let class = seed::class_of(&d.name).to_string();
        let mut dm = DeviceMapping {
            article: d.article.clone(),
            firmware: d.firmware.clone(),
            name: d.name.clone(),
            instance: d.instance.clone(),
            ..Default::default()
        };
        // Two fields of one device that seed to the same path would just
        // coalesce to whichever arrives last, which is a mapping file that
        // silently disagrees with itself. Real devices do this: a battery
        // reports the same six measurements once for its cluster and once for
        // itself, and an alternator reports battery voltage in both its Battery
        // and Shunt groups. The first field id wins and the rest are left out
        // for a human to add deliberately if they want them.
        let mut taken: HashMap<String, FieldId> = HashMap::new();
        let mut ordered: Vec<_> = d.fields.iter().filter(|f| f.menu == MENU).collect();
        ordered.sort_by_key(|f| f.id);
        for f in ordered {
            let Some((s, _tier)) = seed::suggest_best(
                &d.article,
                &d.firmware,
                &class,
                &d.instance,
                f.id,
                &f.name,
                &f.unit,
            ) else {
                continue;
            };
            if let Some(first) = taken.get(s.path.as_str()) {
                log::debug!(
                    "{}: {} would publish to {}, already taken by {}; skipped",
                    d.name,
                    field_key(f.id),
                    s.path,
                    field_key(*first)
                );
                continue;
            }
            taken.insert(s.path.clone(), f.id);
            dm.fields.insert(
                field_key(f.id),
                FieldMapping {
                    path: s.path,
                    invert: s.invert,
                    ..Default::default()
                },
            );
        }
        if !dm.fields.is_empty() {
            m.devices.insert(d.serial.clone(), dm);
        }
    }
    m
}

/// What the editor pre-fills for a field with no suggestion: the node of a
/// path already mapped on this device, so the second field of a solar
/// charger does not need `electrical.solar.solar-chg.` typed again, else
/// just `electrical.`.
pub fn path_prefix(dm: Option<&DeviceMapping>) -> String {
    let node = dm.and_then(|d| d.fields.values().find_map(|f| signalk::node_of(&f.path)));
    match node {
        Some(n) => format!("{n}."),
        None => "electrical.".into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mapping::field_key;

    pub(crate) fn dev(serial: &str, name: &str, fields: &[(FieldId, &str, &str)]) -> DeviceRec {
        DeviceRec {
            id: 0x100000 + serial.len() as u32,
            serial: serial.into(),
            article: "66026000".into(),
            name: name.into(),
            firmware: "2.14".into(),
            instance: seed::instance_of(name, 0x100000),
            fields: fields
                .iter()
                .map(|(i, n, u)| FieldRec {
                    id: *i,
                    name: n.to_string(),
                    unit: u.to_string(),
                    options: Vec::new(),
                    writable: false,
                    menu: MENU,
                    group: "Battery".into(),
                })
                .collect(),
            menus: vec![MENU],
        }
    }

    /// The MLI Ultra field names that the old per-class table silently dropped.
    pub(crate) fn mli() -> DeviceRec {
        dev(
            "MLI-1",
            "BAT 24V Service",
            &[
                (0x000, "State of charge", "%"),
                (0x001, "Voltage", "V"),
                (0x002, "Current", "A"),
                (0x005, "Temperature", "\u{b0}C"),
                (0x022, "Relay close", ""),
            ],
        )
    }

    fn errors(r: &Resolved) -> Vec<String> {
        r.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.to_string())
            .collect()
    }

    /// Two devices that both advertise as `CHG` with unrelated field sets. The
    /// class-and-name table maps neither; the article-keyed database maps both,
    /// differently, which is the case that motivated #12.
    #[test]
    fn seeding_tells_the_two_charger_articles_apart() {
        let mut mass = dev(
            "MASS-1",
            "CHG 24V Ch.U4-1",
            &[
                (0x00E, "Battery voltage", "V"),
                (0x00F, "Battery current", "A"),
            ],
        );
        mass.article = "40021006".into();
        mass.firmware = "7.9".into();
        // The renamed outputs from the boat in #6.
        let mut cm = dev(
            "CM-1",
            "CHG 12V ChargerE",
            &[(0x002, "Eng.batt", "V"), (0x004, "Gen.batt", "V")],
        );
        cm.article = "44010250".into();
        cm.firmware = "0.5".into();

        let m = seed_mapping(&[mass, cm]);
        assert_eq!(
            m.devices["MASS-1"].fields[&field_key(0x00E)].path,
            "electrical.chargers.24v-ch-u4-1.voltage"
        );
        assert_eq!(
            m.devices["CM-1"].fields[&field_key(0x002)].path,
            "electrical.chargers.12v-chargere.output.1.voltage"
        );
    }

    #[test]
    fn seeding_covers_the_battery_names_the_old_table_missed() {
        let m = seed_mapping(&[mli()]);
        let d = &m.devices["MLI-1"];
        assert_eq!(
            d.fields[&field_key(0x001)].path,
            "electrical.batteries.24v-service.voltage"
        );
        assert_eq!(
            d.fields[&field_key(0x005)].path,
            "electrical.batteries.24v-service.temperature"
        );
        // A relay has no Signal K home, so it is simply absent.
        assert!(!d.fields.contains_key(&field_key(0x022)));
    }

    /// Found by deploying onto a live boat: a battery reports the same six
    /// measurements once for its cluster and once for itself, so the seed
    /// produced two fields writing the same Signal K path. They would coalesce
    /// to whichever arrived last, giving a file that silently disagrees with
    /// itself. The lowest field id wins; the rest are left for a human to add
    /// deliberately.
    #[test]
    fn a_device_never_seeds_two_fields_onto_one_path() {
        let d = dev(
            "MLI-CLUSTER",
            "BAT Main Batt",
            &[
                // Cluster group.
                (0x000, "State of charge", "%"),
                (0x001, "Battery", "V"),
                (0x005, "Battery", "\u{b0}C"),
                // The device's own battery group: same measurements again.
                (0x088, "State of charge", "%"),
                (0x08B, "Battery", "V"),
                (0x08D, "Battery", "\u{b0}C"),
            ],
        );
        let m = seed_mapping(&[d]);
        let f = &m.devices["MLI-CLUSTER"].fields;
        let paths: Vec<&str> = f.values().map(|v| v.path.as_str()).collect();
        let unique: HashSet<&str> = paths.iter().copied().collect();
        assert_eq!(paths.len(), unique.len(), "duplicate paths: {paths:?}");
        // The lower id of each pair survives.
        assert!(f.contains_key(&field_key(0x001)));
        assert!(!f.contains_key(&field_key(0x08B)));
    }

    /// The alternator case, which the old code documented as harmless: battery
    /// voltage appears in both the Battery and Shunt groups.
    #[test]
    fn the_alternators_repeated_battery_reading_is_seeded_once() {
        let mut d = dev(
            "APR-1",
            "APR Alternator",
            &[
                (0x006, "Battery voltage", "V"),
                (0x014, "Battery voltage", "V"),
            ],
        );
        d.article = "45512000".into();
        let m = seed_mapping(&[d]);
        let f = &m.devices["APR-1"].fields;
        assert_eq!(f.len(), 1);
        assert!(f.contains_key(&field_key(0x006)));
    }

    #[test]
    fn seeding_records_identity_for_later_editing() {
        let m = seed_mapping(&[mli()]);
        let d = &m.devices["MLI-1"];
        assert_eq!(d.article, "66026000");
        assert_eq!(d.firmware, "2.14");
        assert_eq!(d.name, "BAT 24V Service");
        assert_eq!(d.instance, "24v-service");
    }

    /// Configuration fields are settings, not measurements: they are never
    /// seeded, even once discovered.
    #[test]
    fn seeding_only_proposes_monitoring_fields() {
        let mut d = mli();
        d.fields.push(FieldRec {
            id: 0x101,
            name: "Voltage".into(),
            unit: "V".into(),
            options: vec![],
            writable: true,
            menu: Menu::Configuration,
            group: "Setup".into(),
        });
        let m = seed_mapping(&[d]);
        assert!(!m.devices["MLI-1"].fields.contains_key(&field_key(0x101)));
    }

    #[test]
    fn a_device_with_no_serial_cannot_be_keyed_and_is_skipped() {
        let mut d = mli();
        d.serial = String::new();
        assert!(seed_mapping(&[d]).is_empty());
    }

    #[test]
    fn resolve_derives_the_conversion_from_the_units() {
        let devices = vec![mli()];
        let m = seed_mapping(&devices);
        let r = resolve(&devices, &m);
        assert!(errors(&r).is_empty(), "{:?}", errors(&r));
        let id = devices[0].id;
        // Celsius into a kelvin leaf.
        let t = &r.emit[&(id, 0x005)];
        assert!((t.plan.conv.apply(20.0) - 293.15).abs() < 1e-9);
        assert_eq!(t.plan.unit, Some("K"));
        // Percent into a ratio leaf.
        let soc = &r.emit[&(id, 0x000)];
        assert!((soc.plan.conv.apply(87.0) - 0.87).abs() < 1e-9);
        // Volts into a volts leaf.
        assert!(r.emit[&(id, 0x001)].plan.conv.is_identity());
    }

    /// Every entry the bus cannot honour is skipped *and* explained, with
    /// the device, field and path in the message, since the same text lands
    /// in the daemon's log and in the plugin's editor.
    #[test]
    fn resolve_skips_entries_the_bus_cannot_honour_and_says_why() {
        let devices = vec![mli()];
        let mut m = Mapping::new();
        let mut dm = DeviceMapping {
            firmware: "2.99".into(),
            ..Default::default()
        };
        // A field this device does not have.
        dm.fields.insert(
            field_key(0x0FF),
            FieldMapping {
                path: "electrical.batteries.x.voltage".into(),
                ..Default::default()
            },
        );
        // A field whose unit cannot reach the target leaf.
        dm.fields.insert(
            field_key(0x002),
            FieldMapping {
                path: "electrical.batteries.x.temperature".into(),
                ..Default::default()
            },
        );
        // A key that is not a field id at all.
        dm.fields.insert(
            "banana".into(),
            FieldMapping {
                path: "electrical.batteries.x.current".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        // An entire device that is not on the bus.
        m.devices.insert("GHOST".into(), DeviceMapping::default());

        let r = resolve(&devices, &m);
        assert!(r.emit.is_empty(), "nothing publishable should survive");
        let e = errors(&r);
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(
            e.iter()
                .any(|s| s.contains("0x0FF") && s.contains("no such field")),
            "{e:?}"
        );
        assert!(
            e.iter().any(|s| s.contains("0x002")
                && s.contains("electrical.batteries.x.temperature")
                && s.contains("cannot be converted")),
            "{e:?}"
        );
        assert!(e.iter().any(|s| s.contains("banana")), "{e:?}");
        // The absent device is information, the firmware mismatch a warning.
        assert_eq!(r.count(Severity::Info), 1);
        assert_eq!(r.count(Severity::Warning), 1);
        let ghost = r.diagnostics.iter().find(|d| d.serial == "GHOST").unwrap();
        assert_eq!(ghost.severity, Severity::Info);
        assert!(ghost.to_string().starts_with("GHOST:"), "{ghost}");
    }

    /// A `put` entry publishes like any other, and carries the flag through
    /// so the stream's consumer can register a handler. On a read-only field
    /// it is a warning, not a refusal: an installer login makes it writable.
    #[test]
    fn a_put_entry_publishes_and_warns_when_the_field_is_read_only() {
        let mut d = mli();
        d.fields.push(FieldRec {
            id: 0x013,
            name: "Inverter".into(),
            unit: String::new(),
            options: vec![],
            writable: false,
            menu: Menu::Configuration,
            group: "Inverter".into(),
        });
        let devices = vec![d];
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x013),
            FieldMapping {
                path: "electrical.inverters.x.enabled".into(),
                put: true,
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        let r = resolve(&devices, &m);
        let e = &r.emit[&(devices[0].id, 0x013)];
        assert!(e.put);
        assert_eq!(r.count(Severity::Error), 0);
        assert_eq!(r.count(Severity::Warning), 1);
        assert!(r.diagnostics[0].message.contains("read-only"));
    }

    /// The ids a mapping names that the record has not discovered yet — what
    /// tells the daemon to look at Configuration before resolving.
    #[test]
    fn unknown_fields_lists_what_the_record_lacks() {
        let d = mli();
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        for id in [0x001u16, 0x013, 0x014] {
            dm.fields.insert(
                field_key(id),
                FieldMapping {
                    path: format!("x.y.{id}"),
                    ..Default::default()
                },
            );
        }
        m.devices.insert("MLI-1".into(), dm);
        let mut u = unknown_fields(&d, &m);
        u.sort();
        assert_eq!(u, vec![0x013, 0x014]);
        assert!(unknown_fields(&d, &Mapping::new()).is_empty());
    }

    /// Merging a second menu adds its fields without disturbing the first
    /// menu's, and an id both report belongs to whoever reported it first.
    #[test]
    fn merging_a_menu_is_additive_and_first_wins() {
        let mut d = mli();
        let groups = vec![GroupInfo {
            id: 7,
            name: "Setup".into(),
            menu: Menu::Configuration,
            fields: vec![
                masterbus::FieldInfo {
                    index: 0x001, // already known from Monitoring
                    name: "Voltage setpoint".into(),
                    unit: "V".into(),
                    viz_type: masterbus::VisualizationType::Float,
                    writeable: true,
                    eventable: false,
                    min: 0.0,
                    max: 30.0,
                    step: 0.1,
                    options: vec![],
                },
                masterbus::FieldInfo {
                    index: 0x040,
                    name: "Charger".into(),
                    unit: String::new(),
                    viz_type: masterbus::VisualizationType::CheckBox,
                    writeable: true,
                    eventable: false,
                    min: 0.0,
                    max: 1.0,
                    step: 1.0,
                    options: vec![],
                },
            ],
        }];
        d.merge_groups(Menu::Configuration, groups);
        assert_eq!(d.menus, vec![Menu::Monitoring, Menu::Configuration]);
        assert_eq!(d.field(0x001).unwrap().name, "Voltage");
        let f = d.field(0x040).unwrap();
        assert!(f.writable);
        assert_eq!(f.menu, Menu::Configuration);
        assert_eq!(f.group, "Setup");
        assert!(d.field_ids().contains(&0x040));
    }

    #[test]
    fn menu_names_round_trip() {
        for m in [Menu::Monitoring, Menu::Configuration, Menu::Service] {
            assert_eq!(menu_by_name(menu_name(m)), Some(m));
        }
        assert_eq!(menu_by_name("alarm"), None);
    }

    #[test]
    fn the_prefix_follows_what_the_device_already_publishes() {
        assert_eq!(path_prefix(None), "electrical.");
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x004),
            FieldMapping {
                path: "electrical.solar.solar-chg.panelVoltage".into(),
                ..Default::default()
            },
        );
        assert_eq!(path_prefix(Some(&dm)), "electrical.solar.solar-chg.");
    }
}
