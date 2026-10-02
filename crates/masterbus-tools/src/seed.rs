//! Path *suggestions* — the old per-class name table, demoted.
//!
//! Until issue #12 this table ran unattended in production, which is what made
//! it dangerous: a missing entry published nothing and a wrong entry published
//! a wrong number, both silently. The knowledge in it is fine. Its authority
//! was not.
//!
//! Here it is a suggestion source with no authority at all. It seeds a new
//! `mapping.json` on first run, so an install that worked before keeps working
//! and migrates itself, and it backs the `+` key in the TUI's mapping editor,
//! where a wrong guess costs one keystroke to correct.
//!
//! It matches on the device-class prefix and the field's name and unit, which
//! are exactly the things a real bus was shown to vary. Treat every answer as
//! a proposal to a human, never as a fact.

use masterbus::{DeviceId, FieldId};

use crate::signalk;

use crate::database;

/// A proposed mapping for one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Full Signal K path, instance already substituted.
    pub path: String,
    /// Whether the field's boolean sense is inverted (`Standby` → `enabled`).
    pub invert: bool,
}

impl Suggestion {
    fn path(p: String) -> Option<Suggestion> {
        Some(Suggestion {
            path: p,
            invert: false,
        })
    }
    fn inverted(p: String) -> Option<Suggestion> {
        Some(Suggestion {
            path: p,
            invert: true,
        })
    }
}

/// The class word of a device name: its first whitespace-separated token.
///
/// A convention, not a guarantee. An installer who renames a device without
/// keeping the prefix gets no suggestions, which is the correct failure for a
/// heuristic.
pub fn class_of(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or("")
}

/// The Signal K instance id proposed for a device: its name minus the leading
/// class word, lowercased and reduced to path-safe characters.
pub fn instance_of(name: &str, addr: DeviceId) -> String {
    let label = name
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    if !label.is_empty() {
        sanitize(&label)
    } else if !name.trim().is_empty() {
        sanitize(name)
    } else {
        format!("{addr:06x}")
    }
}

/// Lowercase and keep only Signal K path-segment-safe characters (lowercase
/// reads more idiomatically in Signal K paths).
pub fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "0".into()
    } else {
        cleaned
    }
}

/// Where a suggestion came from, so the editor can say how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// An exact entry for this article *and* firmware in the bundled database.
    ModelFirmware,
    /// An entry for this article in the bundled database.
    Model,
    /// The per-class name heuristics below.
    Name,
    /// Built from the field's group, name and unit; see [`build`].
    Built,
}

impl Tier {
    /// A short phrase for the editor's prompt.
    pub fn describe(self) -> &'static str {
        match self {
            Tier::ModelFirmware => "known for this model and firmware",
            Tier::Model => "known for this model",
            Tier::Name => "guessed from the device class and field name",
            Tier::Built => "built from the group, field name and unit; check it",
        }
    }
}

/// What the suggestion machinery knows about one field.
#[derive(Debug, Clone, Copy)]
pub struct FieldCtx<'a> {
    /// Channel-aware field id.
    pub id: FieldId,
    /// Name, as the installer left it.
    pub name: &'a str,
    /// Unit, as the device reports it (may be empty).
    pub unit: &'a str,
    /// The group the field sits in (`Channels`, `AC inputs`), or empty.
    pub group: &'a str,
}

/// Propose a Signal K path for a field, best evidence first.
///
/// The bundled per-model database is consulted before the name heuristics,
/// because it is keyed on the article number and so can tell apart models the
/// names cannot — two charger articles on the bus in #6 both advertise as
/// `CHG` with unrelated field sets.
///
/// `None` means "no idea", which is the common case and is not an error. This
/// is what seeds a first `mapping.json`, so it stays with evidence; the editor
/// asks [`suggest_or_build`], which also builds a path when there is none.
pub fn suggest_best(
    article: &str,
    firmware: &str,
    class: &str,
    instance: &str,
    f: &FieldCtx,
) -> Option<(Suggestion, Tier)> {
    if let Some(k) = database::lookup(article, firmware, f.id, instance) {
        let tier = if k.exact_firmware {
            Tier::ModelFirmware
        } else {
            Tier::Model
        };
        return Some((k.suggestion, tier));
    }
    suggest(class, instance, f).map(|s| (s, Tier::Name))
}

/// [`suggest_best`], else a path [built](build) from the field's group, name
/// and unit. For the mapping editor, where a human sees it before it is
/// saved: typing `electrical.chargers.chg-inv-sol.ac-inputs.mains.voltage`
/// from scratch is what nobody finishes for a 30-field device, and fixing a
/// built path is quick. Never for seeding, which publishes unattended.
///
/// `node` is where this device's mapped paths already live, if anywhere
/// (see [`crate::publish::mapped_node`]); a built path joins them there
/// rather than starting a second tree for the same device.
pub fn suggest_or_build(
    article: &str,
    firmware: &str,
    class: &str,
    instance: &str,
    node: Option<&str>,
    f: &FieldCtx,
) -> Option<(Suggestion, Tier)> {
    suggest_best(article, firmware, class, instance, f).or_else(|| {
        let base = node
            .map(str::to_string)
            .unwrap_or_else(|| format!("electrical.{}.{instance}", category_of(class)));
        build(&base, f).map(|s| (s, Tier::Built))
    })
}

/// Propose a Signal K path from the device class and the field's name, unit
/// and group.
///
/// The weakest evidence-based tier. Prefer [`suggest_best`], which consults
/// the per-model database first. Names are matched regardless of case:
/// firmware is not consistent (`State of charge` on a battery, `State of
/// Charge` on a MasterShunt).
pub fn suggest(class: &str, instance: &str, f: &FieldCtx) -> Option<Suggestion> {
    let deg = "\u{b0}C";
    let unit = f.unit;
    let ut = unit.trim();
    let lname = f.name.trim().to_lowercase();
    let name = lname.as_str();
    match class {
        // Battery monitors → electrical.batteries.<instance>
        // MSH, the MasterShunt, reports the BTM family's fields under its own
        // class word.
        "BAT" | "MSH" => {
            let b = format!("electrical.batteries.{instance}");
            match (name, unit) {
                ("state of charge", _) => Suggestion::path(format!("{b}.capacity.stateOfCharge")),
                ("time remaining" | "remaining", _) => {
                    Suggestion::path(format!("{b}.capacity.timeRemaining"))
                }
                ("cap. consumed", _) => {
                    Suggestion::path(format!("{b}.capacity.dischargeSinceFull"))
                }
                // Two naming conventions for the same three measurements: the
                // BTM family calls them "Battery", the MLI Ultra family calls
                // them "Voltage" / "Current" / "Temperature".
                ("battery" | "voltage", "V") => Suggestion::path(format!("{b}.voltage")),
                ("battery" | "current", "A") => Suggestion::path(format!("{b}.current")),
                ("battery" | "temperature", d) if d == deg => {
                    Suggestion::path(format!("{b}.temperature"))
                }
                _ => None,
            }
        }
        // CombiMaster: an inverter and a charger in one box.
        "CMR" => {
            let inv = format!("electrical.inverters.{instance}");
            let chg = format!("electrical.chargers.{instance}");
            match (name, unit) {
                ("battery voltage", "V") => Suggestion::path(format!("{inv}.dc.voltage")),
                ("battery current", "A") => Suggestion::path(format!("{inv}.dc.current")),
                ("battery temp.", d) if d == deg => {
                    Suggestion::path(format!("{inv}.dc.temperature"))
                }
                ("output voltage", "V") => Suggestion::path(format!("{inv}.ac.voltage")),
                ("output power", "W") => Suggestion::path(format!("{inv}.ac.power")),
                ("output frequency", "Hz") => Suggestion::path(format!("{inv}.ac.frequency")),
                ("input voltage", "V") => Suggestion::path(format!("{chg}.acin.voltage")),
                ("input current", "A") => Suggestion::path(format!("{chg}.acin.current")),
                ("input frequency", "Hz") => Suggestion::path(format!("{chg}.acin.frequency")),
                ("ac in limit", "A") => Suggestion::path(format!("{chg}.acin.currentLimit")),
                ("inverter", _) => Suggestion::path(format!("{inv}.enabled")),
                ("charger", _) => Suggestion::path(format!("{chg}.enabled")),
                _ => None,
            }
        }
        // MAC — DC-DC charger. Several of its monitoring fields report an empty
        // unit, so the unit is only used where it disambiguates.
        "MAC" => {
            let chg = format!("electrical.chargers.{instance}");
            match (name, ut) {
                ("output voltage", "V" | "") => Suggestion::path(format!("{chg}.voltage")),
                ("output current", "A" | "") => Suggestion::path(format!("{chg}.current")),
                ("input voltage", "V" | "") => Suggestion::path(format!("{chg}.input.voltage")),
                ("input current", "A" | "") => Suggestion::path(format!("{chg}.input.current")),
                ("bat. volt sense", "V" | "") => Suggestion::path(format!("{chg}.voltageSense")),
                // "Device" and "Battery" are generic; only °C tells them apart.
                ("device", d) if d == deg => Suggestion::path(format!("{chg}.temperature")),
                ("battery", d) if d == deg => {
                    Suggestion::path(format!("{chg}.battery.temperature"))
                }
                ("device state", _) => Suggestion::path(format!("{chg}.deviceMode")),
                ("charge state", _) => Suggestion::path(format!("{chg}.chargingMode")),
                // "Standby" off means the charger is running.
                ("standby", _) => Suggestion::inverted(format!("{chg}.enabled")),
                // A checkbox that reads `on` while the charger runs.
                ("on/standby", _) => Suggestion::path(format!("{chg}.enabled")),
                _ => None,
            }
        }
        // APR — Alpha Pro alternator regulator.
        "APR" => {
            let alt = format!("electrical.alternators.{instance}");
            match (name, unit) {
                ("alternator volt.", "V") => Suggestion::path(format!("{alt}.voltage")),
                ("sense voltage", "V") => Suggestion::path(format!("{alt}.voltageSense")),
                ("field current", "A") => Suggestion::path(format!("{alt}.field.current")),
                ("alternator temp.", d) if d == deg => {
                    Suggestion::path(format!("{alt}.temperature"))
                }
                ("alternator shaft", "rpm" | "RPM") => {
                    Suggestion::path(format!("{alt}.revolutions"))
                }
                ("engine shaft", "rpm" | "RPM") => {
                    Suggestion::path(format!("{alt}.engine.revolutions"))
                }
                ("charger state", _) => Suggestion::path(format!("{alt}.chargingMode")),
                ("state of charge", "%") => {
                    Suggestion::path(format!("{alt}.battery.stateOfCharge"))
                }
                ("battery voltage", "V") => Suggestion::path(format!("{alt}.battery.voltage")),
                ("battery current", "A") => Suggestion::path(format!("{alt}.battery.current")),
                ("battery temp.", d) if d == deg => {
                    Suggestion::path(format!("{alt}.battery.temperature"))
                }
                _ => None,
            }
        }
        // INT — a MasterBus interface driving one output. Its `State`
        // (Standby / Activated) is a switch's on/off, which the spec types as
        // a boolean `state`; the labels classify on their own.
        "INT" => match name {
            "state" => Suggestion::path(format!("electrical.switches.{instance}.state")),
            _ => None,
        },
        // DSD — digital switching. Every checkbox in its `Channels` group is
        // one switched output, named by the installer. The channel becomes a
        // node under the device, so "copy to same model" still finds the
        // instance in the third segment.
        "DSD" if f.group.eq_ignore_ascii_case("Channels") && ut.is_empty() => {
            let ch = segment(f.name)?;
            Suggestion::path(format!("electrical.switches.{instance}.{ch}.state"))
        }
        // ISO — an isolation transformer: the shore side and the boat side
        // of one AC supply, each a single phase.
        "ISO" => {
            let side = match name {
                "shore" | "cos phi" => "shore",
                "output" => "output",
                _ => return None,
            };
            let leaf = match (name, ut) {
                ("cos phi", _) => "powerFactor",
                (_, "V") => "lineNeutralVoltage",
                (_, "A") => "current",
                (_, "W" | "kW") => "realPower",
                (_, "Hz") => "frequency",
                _ => return None,
            };
            Suggestion::path(format!("electrical.ac.{instance}.{side}.phase.A.{leaf}"))
        }
        _ => None,
    }
}

/// The Signal K category a device class publishes into, for [`build`].
fn category_of(class: &str) -> String {
    match class {
        "BAT" | "MSH" => "batteries".into(),
        "CHG" | "MAC" => "chargers".into(),
        "INV" | "CMR" | "MCU" => "inverters".into(),
        "APR" => "alternators".into(),
        "SOL" => "solar".into(),
        "ISO" => "ac".into(),
        // Only what switches an output: a DC distributor's or an input
        // module's `state` is health, not the on/off a switch's `state` is.
        "INT" | "DSD" => "switches".into(),
        "" => "devices".into(),
        other => segment(other).unwrap_or_else(|| "devices".into()),
    }
}

/// The Signal K leaf a device unit stands for, where the unit says it alone.
fn unit_leaf(unit: &str) -> Option<&'static str> {
    Some(match unit.trim() {
        "V" => "voltage",
        "A" => "current",
        "\u{b0}C" | "\u{b0}F" | "K" => "temperature",
        "W" | "kW" => "power",
        "Hz" => "frequency",
        "rpm" | "RPM" => "revolutions",
        _ => return None,
    })
}

/// A path built from what the device says about a field when no rule knows
/// it: `<node>.<group>.<name>.<leaf>`, the leaf from the unit where the unit
/// names one (`Mains` in V → `…mains.voltage`) and from the name otherwise
/// (`Device state` → `…device-state`). [`suggest_or_build`] picks the node:
/// `electrical.<category>.<instance>`, the category following the class word.
///
/// Proposals for a human to trim, not facts: the group and name are the
/// installer's.
pub fn build(node: &str, f: &FieldCtx) -> Option<Suggestion> {
    let mut path = node.to_string();
    let group = segment(f.group);
    let name = segment(f.name);
    if let Some(g) = &group
        && name.as_deref() != Some(g.as_str())
    {
        path.push('.');
        path.push_str(g);
    }
    match (unit_leaf(f.unit), name) {
        (Some(leaf), Some(n)) => path = format!("{path}.{n}.{leaf}"),
        (Some(leaf), None) => path = format!("{path}.{leaf}"),
        (None, Some(n)) => path = format!("{path}.{n}"),
        (None, None) => return None,
    }
    // Belt and braces: a unit the leaf cannot take would be refused on save.
    crate::units::conversion(f.unit, signalk::leaf_unit(&path))?;
    Suggestion::path(path)
}

/// One path segment from free text: lowercase, runs of anything but letters
/// and digits become one `-`, no `-` at either end. `None` when nothing is
/// left.
pub fn segment(s: &str) -> Option<String> {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    (!out.is_empty()).then_some(out)
}

/// Keep a proposed path from landing on one another field of the same device
/// already publishes to. Installers repeat channel names (`Spare`, twice, on
/// one switch panel), and two fields on one path coalesce into whichever
/// updates last. The field id goes onto the segment before the leaf:
/// `…spare.state` becomes `…spare-0x015.state`.
pub fn unique<'a>(
    path: String,
    field: FieldId,
    mut taken: impl Iterator<Item = (FieldId, &'a str)>,
) -> String {
    if !taken.any(|(id, p)| id != field && p == path) {
        return path;
    }
    let tag = crate::mapping::field_key(field).to_lowercase();
    match path.rsplit_once('.') {
        Some((head, leaf)) => format!("{head}-{tag}.{leaf}"),
        None => format!("{path}-{tag}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signalk::leaf_unit;
    use crate::units::conversion;

    fn ctx<'a>(name: &'a str, unit: &'a str) -> FieldCtx<'a> {
        FieldCtx {
            id: 0x001,
            name,
            unit,
            group: "",
        }
    }

    fn path_of(class: &str, name: &str, unit: &str) -> Option<String> {
        suggest(class, "x", &ctx(name, unit)).map(|s| s.path)
    }

    #[test]
    fn battery_families_that_name_the_same_field_differently_both_map() {
        // BTM family.
        assert_eq!(
            path_of("BAT", "Battery", "V").as_deref(),
            Some("electrical.batteries.x.voltage")
        );
        // MLI Ultra family — the case that silently published nothing before.
        assert_eq!(
            path_of("BAT", "Voltage", "V").as_deref(),
            Some("electrical.batteries.x.voltage")
        );
        assert_eq!(
            path_of("BAT", "Current", "A").as_deref(),
            Some("electrical.batteries.x.current")
        );
        assert_eq!(
            path_of("BAT", "Temperature", "\u{b0}C").as_deref(),
            Some("electrical.batteries.x.temperature")
        );
    }

    #[test]
    fn generic_names_still_need_their_unit_to_disambiguate() {
        // A "Battery" float in volts is not a temperature.
        assert_eq!(path_of("MAC", "Battery", "V"), None);
        assert_eq!(
            path_of("MAC", "Battery", "\u{b0}C").as_deref(),
            Some("electrical.chargers.x.battery.temperature")
        );
    }

    #[test]
    fn standby_is_the_one_inverted_boolean() {
        let s = suggest("MAC", "x", &ctx("Standby", "")).unwrap();
        assert_eq!(s.path, "electrical.chargers.x.enabled");
        assert!(s.invert);
        assert!(!suggest("CMR", "x", &ctx("Charger", "")).unwrap().invert);
    }

    #[test]
    fn mac_fields_with_no_unit_still_map() {
        assert_eq!(
            path_of("MAC", "Output voltage", "").as_deref(),
            Some("electrical.chargers.x.voltage")
        );
    }

    #[test]
    fn alternator_shaft_speed_accepts_either_spelling() {
        assert!(path_of("APR", "Engine shaft", "rpm").is_some());
        assert!(path_of("APR", "Engine shaft", "RPM").is_some());
    }

    /// The guarantee that lets the mapping file omit scale factors: every path
    /// this table proposes must be reachable from the field's own unit.
    #[test]
    fn every_suggestion_has_a_derivable_conversion() {
        let cases: &[(&str, &str, &str)] = &[
            ("BAT", "State of charge", "%"),
            ("BAT", "Time remaining", ""),
            ("BAT", "Cap. consumed", "Ah"),
            ("BAT", "Voltage", "V"),
            ("BAT", "Current", "A"),
            ("BAT", "Temperature", "\u{b0}C"),
            ("BAT", "Battery", "V"),
            ("BAT", "Battery", "A"),
            ("BAT", "Battery", "\u{b0}C"),
            ("CMR", "Battery voltage", "V"),
            ("CMR", "Battery temp.", "\u{b0}C"),
            ("CMR", "Output power", "W"),
            ("CMR", "Output frequency", "Hz"),
            ("CMR", "AC IN limit", "A"),
            ("CMR", "Inverter", ""),
            ("MAC", "Output voltage", ""),
            ("MAC", "Device", "\u{b0}C"),
            ("MAC", "Charge state", ""),
            ("MAC", "Standby", ""),
            ("APR", "Alternator shaft", "rpm"),
            ("APR", "Engine shaft", "RPM"),
            ("APR", "State of charge", "%"),
            ("APR", "Alternator temp.", "\u{b0}C"),
            ("APR", "Charger state", ""),
            ("MSH", "State of Charge", "%"),
            ("MSH", "Remaining", ""),
            ("MSH", "Cap. consumed", "Ah"),
            ("MSH", "Battery", "\u{b0}C"),
            ("MAC", "On/Standby", ""),
            ("INT", "State", ""),
            ("ISO", "Shore", "V"),
            ("ISO", "Shore", "kW"),
            ("ISO", "Shore", "Hz"),
            ("ISO", "Output", "A"),
            ("ISO", "cos phi", ""),
        ];
        for (class, name, unit) in cases {
            let s = suggest(class, "x", &ctx(name, unit))
                .unwrap_or_else(|| panic!("{class}/{name}/{unit:?} should suggest a path"));
            assert!(
                conversion(unit, leaf_unit(&s.path)).is_some(),
                "{class}/{name}/{unit:?} → {} has no derivable conversion",
                s.path
            );
        }
    }

    #[test]
    fn instance_strips_the_class_word_and_sanitizes() {
        assert_eq!(instance_of("BAT 24V Aft Serv", 0x123456), "24v-aft-serv");
        assert_eq!(instance_of("CHG 12V ChargerE", 0x123456), "12v-chargere");
        // A single-word name keeps the whole thing.
        assert_eq!(instance_of("Repeater", 0x123456), "repeater");
        // No name at all falls back to the address.
        assert_eq!(instance_of("", 0x123456), "123456");
    }

    /// The database must win over the name heuristics: on the ChargeMaster the
    /// installer renamed 0x002 to "Eng.batt", which the name table would map to
    /// nothing at all, while the article-keyed entry knows it is output 1.
    #[test]
    fn the_model_database_outranks_the_name_guess() {
        let (s, tier) = suggest_best(
            "44010250",
            "0.5",
            "CHG",
            "chargere",
            &FieldCtx {
                id: 0x002,
                ..ctx("Eng.batt", "V")
            },
        )
        .expect("the database knows this field");
        assert_eq!(s.path, "electrical.chargers.chargere.output.1.voltage");
        assert_eq!(tier, Tier::Model);
    }

    /// A model the database does not carry still gets the name heuristic.
    #[test]
    fn an_unknown_model_falls_through_to_the_name_guess() {
        let (s, tier) = suggest_best("99999999", "1.0", "BAT", "house", &ctx("Voltage", "V"))
            .expect("the name table knows this one");
        assert_eq!(s.path, "electrical.batteries.house.voltage");
        assert_eq!(tier, Tier::Name);
    }

    #[test]
    fn a_field_neither_tier_knows_suggests_nothing() {
        assert!(suggest_best("99999999", "1.0", "BAT", "x", &ctx("Relay close", "")).is_none());
    }

    #[test]
    fn class_is_the_first_word() {
        assert_eq!(class_of("BAT 24V Service"), "BAT");
        assert_eq!(class_of(""), "");
    }

    // ---- learned from samples/mcu-czone-19-devices.json --------------------

    #[test]
    fn a_mastershunt_is_a_battery_whatever_the_capitals() {
        assert_eq!(
            path_of("MSH", "State of Charge", "%").as_deref(),
            Some("electrical.batteries.x.capacity.stateOfCharge")
        );
        assert_eq!(
            path_of("MSH", "Remaining", "").as_deref(),
            Some("electrical.batteries.x.capacity.timeRemaining")
        );
        assert_eq!(
            path_of("MSH", "Battery", "A").as_deref(),
            Some("electrical.batteries.x.current")
        );
        assert_eq!(
            path_of("BAT", "VOLTAGE", "V").as_deref(),
            Some("electrical.batteries.x.voltage")
        );
    }

    #[test]
    fn on_standby_is_enabled_and_not_inverted() {
        let s = suggest("MAC", "x", &ctx("On/Standby", "")).unwrap();
        assert_eq!(s.path, "electrical.chargers.x.enabled");
        assert!(!s.invert);
    }

    #[test]
    fn an_interface_state_is_a_switch() {
        assert_eq!(
            path_of("INT", "State", "").as_deref(),
            Some("electrical.switches.x.state")
        );
        assert_eq!(path_of("INT", "Override", ""), None);
    }

    #[test]
    fn a_digital_switching_channel_is_a_switch_under_the_device() {
        let f = FieldCtx {
            group: "Channels",
            ..ctx("Bilge Pump Front", "")
        };
        assert_eq!(
            suggest("DSD", "1-power", &f).unwrap().path,
            "electrical.switches.1-power.bilge-pump-front.state"
        );
        // Outside its Channels group nothing is a channel.
        let f = FieldCtx {
            group: "Reset",
            ..ctx("Reset alarms", "")
        };
        assert!(suggest("DSD", "1-power", &f).is_none());
    }

    #[test]
    fn an_isolation_transformer_publishes_both_sides_as_ac() {
        assert_eq!(
            path_of("ISO", "Shore", "V").as_deref(),
            Some("electrical.ac.x.shore.phase.A.lineNeutralVoltage")
        );
        assert_eq!(
            path_of("ISO", "Output", "kW").as_deref(),
            Some("electrical.ac.x.output.phase.A.realPower")
        );
        assert_eq!(
            path_of("ISO", "cos phi", "").as_deref(),
            Some("electrical.ac.x.shore.phase.A.powerFactor")
        );
        assert_eq!(path_of("ISO", "State", ""), None);
    }

    #[test]
    fn a_path_is_built_from_group_name_and_unit() {
        let f = |group, name, unit| FieldCtx {
            group,
            ..ctx(name, unit)
        };
        let b = |class, f: FieldCtx| {
            suggest_or_build("0", "0", class, "chg-inv-sol", None, &f).map(|(s, _)| s.path)
        };
        assert_eq!(
            b("MCU", f("AC inputs", "Generator", "V")).as_deref(),
            Some("electrical.inverters.chg-inv-sol.ac-inputs.generator.voltage")
        );
        // No unit: the name is the leaf.
        assert_eq!(
            b("MCU", f("General", "AC in state", "")).as_deref(),
            Some("electrical.inverters.chg-inv-sol.general.ac-in-state")
        );
        // A group that repeats the name is said once.
        assert_eq!(
            b("MCU", f("Sec. charger", "Sec. charger", "")).as_deref(),
            Some("electrical.inverters.chg-inv-sol.sec-charger")
        );
        // An unknown class becomes its own category.
        assert_eq!(
            b("DIS", f("Power save", "Backlight", "%")).as_deref(),
            Some("electrical.dis.chg-inv-sol.power-save.backlight")
        );
        assert!(b("MCU", f("", "", "")).is_none());
        // A fuse distributor's state is not a switch's on/off.
        assert_eq!(
            b("DCD", f("Device", "State", "")).as_deref(),
            Some("electrical.dcd.chg-inv-sol.device.state")
        );
        // A device already mapped somewhere keeps its paths together.
        let (s, _) = suggest_or_build(
            "0",
            "0",
            "MCU",
            "chg-inv-sol",
            Some("electrical.chargers.ChgInvSol"),
            &f("AC inputs", "Generator", "V"),
        )
        .unwrap();
        assert_eq!(
            s.path,
            "electrical.chargers.ChgInvSol.ac-inputs.generator.voltage"
        );
    }

    #[test]
    fn the_editor_builds_where_nothing_is_known_but_seeding_does_not() {
        let f = FieldCtx {
            group: "AC inputs",
            ..ctx("Generator", "V")
        };
        assert!(suggest_best("99999999", "1.0", "XYZ", "x", &f).is_none());
        let (s, tier) = suggest_or_build("99999999", "1.0", "XYZ", "x", None, &f).unwrap();
        assert_eq!(tier, Tier::Built);
        assert_eq!(s.path, "electrical.xyz.x.ac-inputs.generator.voltage");
    }

    #[test]
    fn segments_are_tidy() {
        assert_eq!(segment("Battery (DC)").as_deref(), Some("battery-dc"));
        assert_eq!(segment("  N2K net 1 ").as_deref(), Some("n2k-net-1"));
        assert_eq!(
            segment("Radar+ SonarHub").as_deref(),
            Some("radar-sonarhub")
        );
        assert_eq!(segment("--"), None);
    }

    #[test]
    fn a_repeated_channel_name_gets_its_field_id() {
        let p = "electrical.switches.x.spare.state".to_string();
        let taken = [(0x009, "electrical.switches.x.spare.state")];
        assert_eq!(
            unique(p.clone(), 0x015, taken.iter().copied()),
            "electrical.switches.x.spare-0x015.state"
        );
        // The field's own entry is not a collision.
        assert_eq!(unique(p.clone(), 0x009, taken.iter().copied()), p);
    }

    /// Every Monitoring field of a real 19-device bus gets a proposal the
    /// editor would accept on save, and no two fields of one device share a
    /// path once [`unique`] has had its say. Reads the dump at run time, so a
    /// packaged crate without `samples/` skips rather than fails.
    #[test]
    fn every_field_of_the_sample_bus_gets_a_path_that_would_save() {
        use crate::mapping::{FieldMapping, field_key};
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../samples/mcu-czone-19-devices.json"
        );
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let dump: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let mut by_tier = std::collections::BTreeMap::<&str, usize>::new();
        for dev in dump["devices"].as_array().unwrap() {
            let s = |k: &str| dev[k].as_str().unwrap_or_default();
            let addr = u32::from_str_radix(s("id").trim_start_matches("0x"), 16).unwrap();
            let instance = instance_of(s("name"), addr);
            let mut taken: Vec<(FieldId, String)> = Vec::new();
            for g in dev["groups"].as_array().unwrap() {
                if g["menu"] != "monitoring" {
                    continue;
                }
                for f in g["fields"].as_array().unwrap() {
                    let fs = |k: &str| f[k].as_str().unwrap_or_default();
                    let id = crate::mapping::parse_field_key(fs("id")).unwrap();
                    let options: Vec<String> = f["options"]
                        .as_array()
                        .map(|o| {
                            o.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let ctx = FieldCtx {
                        id,
                        name: fs("name"),
                        unit: fs("unit"),
                        group: g["name"].as_str().unwrap_or_default(),
                    };
                    let Some((sug, tier)) = suggest_or_build(
                        s("article"),
                        s("firmware"),
                        class_of(s("name")),
                        &instance,
                        None,
                        &ctx,
                    ) else {
                        panic!("{} {}: nothing proposed", s("name"), fs("name"));
                    };
                    let p = unique(sug.path, id, taken.iter().map(|(i, p)| (*i, p.as_str())));
                    let entry = FieldMapping {
                        path: p.clone(),
                        invert: sug.invert,
                        ..Default::default()
                    };
                    match signalk::plan(&p, ctx.unit, &options, &entry) {
                        Ok(_) | Err(signalk::Refusal::Truth { .. }) => {}
                        Err(e) => panic!(
                            "{} {} {}: {p} refused: {e}",
                            s("name"),
                            field_key(id),
                            ctx.name
                        ),
                    }
                    assert!(
                        !taken.iter().any(|(_, q)| *q == p),
                        "{} {}: {p} taken twice",
                        s("name"),
                        ctx.name
                    );
                    taken.push((id, p));
                    *by_tier.entry(tier.describe()).or_default() += 1;
                }
            }
        }
        // The evidence tiers carry the devices this dump taught us about.
        let built = by_tier.get(Tier::Built.describe()).copied().unwrap_or(0);
        let known: usize = by_tier.values().sum::<usize>() - built;
        assert!(known >= 60, "{by_tier:?}");
    }
}
