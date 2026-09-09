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
}

impl Tier {
    /// A short phrase for the editor's prompt.
    pub fn describe(self) -> &'static str {
        match self {
            Tier::ModelFirmware => "known for this model and firmware",
            Tier::Model => "known for this model",
            Tier::Name => "guessed from the device class and field name",
        }
    }
}

/// Propose a Signal K path for a field, best evidence first.
///
/// The bundled per-model database is consulted before the name heuristics,
/// because it is keyed on the article number and so can tell apart models the
/// names cannot — two charger articles on the bus in #6 both advertise as
/// `CHG` with unrelated field sets.
///
/// `None` means "no idea", which is the common case and is not an error.
pub fn suggest_best(
    article: &str,
    firmware: &str,
    class: &str,
    instance: &str,
    field: FieldId,
    name: &str,
    unit: &str,
) -> Option<(Suggestion, Tier)> {
    if let Some(k) = database::lookup(article, firmware, field, instance) {
        let tier = if k.exact_firmware {
            Tier::ModelFirmware
        } else {
            Tier::Model
        };
        return Some((k.suggestion, tier));
    }
    suggest(class, instance, name, unit).map(|s| (s, Tier::Name))
}

/// Propose a Signal K path from the device class, field name and unit alone.
///
/// The weakest tier. Prefer [`suggest_best`], which consults the per-model
/// database first.
pub fn suggest(class: &str, instance: &str, name: &str, unit: &str) -> Option<Suggestion> {
    let deg = "\u{b0}C";
    let ut = unit.trim();
    match class {
        // Battery monitors → electrical.batteries.<instance>
        "BAT" => {
            let b = format!("electrical.batteries.{instance}");
            match (name, unit) {
                ("State of charge", _) => Suggestion::path(format!("{b}.capacity.stateOfCharge")),
                ("Time remaining", _) => Suggestion::path(format!("{b}.capacity.timeRemaining")),
                ("Cap. consumed", _) => {
                    Suggestion::path(format!("{b}.capacity.dischargeSinceFull"))
                }
                // Two naming conventions for the same three measurements: the
                // BTM family calls them "Battery", the MLI Ultra family calls
                // them "Voltage" / "Current" / "Temperature".
                ("Battery" | "Voltage", "V") => Suggestion::path(format!("{b}.voltage")),
                ("Battery" | "Current", "A") => Suggestion::path(format!("{b}.current")),
                ("Battery" | "Temperature", d) if d == deg => {
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
                ("Battery voltage", "V") => Suggestion::path(format!("{inv}.dc.voltage")),
                ("Battery current", "A") => Suggestion::path(format!("{inv}.dc.current")),
                ("Battery temp.", d) if d == deg => {
                    Suggestion::path(format!("{inv}.dc.temperature"))
                }
                ("Output voltage", "V") => Suggestion::path(format!("{inv}.ac.voltage")),
                ("Output power", "W") => Suggestion::path(format!("{inv}.ac.power")),
                ("Output frequency", "Hz") => Suggestion::path(format!("{inv}.ac.frequency")),
                ("Input voltage", "V") => Suggestion::path(format!("{chg}.acin.voltage")),
                ("Input current", "A") => Suggestion::path(format!("{chg}.acin.current")),
                ("Input frequency", "Hz") => Suggestion::path(format!("{chg}.acin.frequency")),
                ("AC IN limit", "A") => Suggestion::path(format!("{chg}.acin.currentLimit")),
                ("Inverter", _) => Suggestion::path(format!("{inv}.enabled")),
                ("Charger", _) => Suggestion::path(format!("{chg}.enabled")),
                _ => None,
            }
        }
        // MAC — DC-DC charger. Several of its monitoring fields report an empty
        // unit, so the unit is only used where it disambiguates.
        "MAC" => {
            let chg = format!("electrical.chargers.{instance}");
            match (name, ut) {
                ("Output voltage", "V" | "") => Suggestion::path(format!("{chg}.voltage")),
                ("Output current", "A" | "") => Suggestion::path(format!("{chg}.current")),
                ("Input voltage", "V" | "") => Suggestion::path(format!("{chg}.input.voltage")),
                ("Input current", "A" | "") => Suggestion::path(format!("{chg}.input.current")),
                ("Bat. volt sense", "V" | "") => Suggestion::path(format!("{chg}.voltageSense")),
                // "Device" and "Battery" are generic; only °C tells them apart.
                ("Device", d) if d == deg => Suggestion::path(format!("{chg}.temperature")),
                ("Battery", d) if d == deg => {
                    Suggestion::path(format!("{chg}.battery.temperature"))
                }
                ("Device state", _) => Suggestion::path(format!("{chg}.deviceMode")),
                ("Charge state", _) => Suggestion::path(format!("{chg}.chargingMode")),
                // "Standby" off means the charger is running.
                ("Standby", _) => Suggestion::inverted(format!("{chg}.enabled")),
                _ => None,
            }
        }
        // APR — Alpha Pro alternator regulator.
        "APR" => {
            let alt = format!("electrical.alternators.{instance}");
            match (name, unit) {
                ("Alternator volt.", "V") => Suggestion::path(format!("{alt}.voltage")),
                ("Sense voltage", "V") => Suggestion::path(format!("{alt}.voltageSense")),
                ("Field current", "A") => Suggestion::path(format!("{alt}.field.current")),
                ("Alternator temp.", d) if d == deg => {
                    Suggestion::path(format!("{alt}.temperature"))
                }
                ("Alternator shaft", "rpm" | "RPM") => {
                    Suggestion::path(format!("{alt}.revolutions"))
                }
                ("Engine shaft", "rpm" | "RPM") => {
                    Suggestion::path(format!("{alt}.engine.revolutions"))
                }
                ("Charger state", _) => Suggestion::path(format!("{alt}.chargingMode")),
                ("State of charge", "%") => {
                    Suggestion::path(format!("{alt}.battery.stateOfCharge"))
                }
                ("Battery voltage", "V") => Suggestion::path(format!("{alt}.battery.voltage")),
                ("Battery current", "A") => Suggestion::path(format!("{alt}.battery.current")),
                ("Battery temp.", d) if d == deg => {
                    Suggestion::path(format!("{alt}.battery.temperature"))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signalk::leaf_unit;
    use crate::units::conversion;

    fn path_of(class: &str, name: &str, unit: &str) -> Option<String> {
        suggest(class, "x", name, unit).map(|s| s.path)
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
        let s = suggest("MAC", "x", "Standby", "").unwrap();
        assert_eq!(s.path, "electrical.chargers.x.enabled");
        assert!(s.invert);
        assert!(!suggest("CMR", "x", "Charger", "").unwrap().invert);
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
        ];
        for (class, name, unit) in cases {
            let s = suggest(class, "x", name, unit)
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
        let (s, tier) = suggest_best("44010250", "0.5", "CHG", "chargere", 0x002, "Eng.batt", "V")
            .expect("the database knows this field");
        assert_eq!(s.path, "electrical.chargers.chargere.output.1.voltage");
        assert_eq!(tier, Tier::Model);
    }

    /// A model the database does not carry still gets the name heuristic.
    #[test]
    fn an_unknown_model_falls_through_to_the_name_guess() {
        let (s, tier) = suggest_best("99999999", "1.0", "BAT", "house", 0x001, "Voltage", "V")
            .expect("the name table knows this one");
        assert_eq!(s.path, "electrical.batteries.house.voltage");
        assert_eq!(tier, Tier::Name);
    }

    #[test]
    fn a_field_neither_tier_knows_suggests_nothing() {
        assert!(suggest_best("99999999", "1.0", "BAT", "x", 0x022, "Relay close", "").is_none());
    }

    #[test]
    fn class_is_the_first_word() {
        assert_eq!(class_of("BAT 24V Service"), "BAT");
        assert_eq!(class_of(""), "");
    }
}
