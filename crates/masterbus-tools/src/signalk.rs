//! The Signal K side of the mapping: what a path *means*, independent of any
//! particular MasterBus device.
//!
//! Unit metadata is derived from the **device's** unit (see
//! [`crate::units::to_si`]), not from the path. The leaf table here is a
//! cross-check and a typing hint: it refuses an ampere field pointed at a
//! `voltage` leaf, and it says which leaves are booleans so an enum such as
//! `Standby` / `On` can be published as `true` / `false` through a truth table.

use std::collections::BTreeMap;

use masterbus::Value;

use crate::mapping::FieldMapping;
use crate::units::{self, Conversion};

/// The SI unit the Signal K specification gives a leaf, keyed on the path's
/// last segment. `None` for a leaf that carries no unit — a string state such
/// as `chargingMode`, a boolean such as `enabled` — and for any leaf this
/// table does not list.
///
/// This is *not* what decides the published unit: that follows from the
/// device's own unit. It only catches a mapping whose device unit cannot reach
/// what the leaf is documented to want, and the one case a device unit cannot
/// express — a leaf that wants joules from a field reporting amp-hours.
pub fn leaf_unit(path: &str) -> Option<&'static str> {
    Some(match leaf_of(path) {
        "stateOfCharge" | "stateOfHealth" => "ratio",
        "timeRemaining" => "s",
        "dischargeSinceFull" => "C",
        "temperature" => "K",
        "voltage" | "voltageSense" | "panelVoltage" | "lineNeutralVoltage" | "lineLineVoltage" => {
            "V"
        }
        "current" | "currentLimit" | "panelCurrent" | "loadCurrent" => "A",
        "power" | "realPower" => "W",
        "frequency" | "revolutions" => "Hz",
        // `yieldToday` is the spec's; `yieldTotal` is this project's sibling
        // for a lifetime energy counter the spec does not have.
        "yieldToday" | "yieldTotal" => "J",
        _ => return None,
    })
}

/// Whether the specification types a leaf as a boolean: `enabled` anywhere,
/// and a switch's `state`.
pub fn leaf_is_boolean(path: &str) -> bool {
    match leaf_of(path) {
        "enabled" => true,
        "state" => path.starts_with("electrical.switches."),
        _ => false,
    }
}

fn leaf_of(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or("")
}

/// The boolean an enum label conventionally means, for the labels seen on
/// MasterBus devices. `None` for a label that could go either way (`Alarm`,
/// `Bulk`), which is when a human has to say.
pub fn truth_of_label(label: &str) -> Option<bool> {
    let l = label.trim().to_ascii_lowercase();
    match l.as_str() {
        "on" | "activated" | "active" | "enabled" | "yes" | "true" | "closed" => Some(true),
        "off" | "standby" | "disabled" | "inactive" | "no" | "false" | "open" => Some(false),
        _ => None,
    }
}

/// A complete truth table for an enum's labels, when every label is one this
/// build knows and both values occur. `Off`/`On` and `Standby`/`On` qualify;
/// `Standby`/`On`/`Alarm` does not, and comes back `None` so the editor asks.
pub fn truth_default(labels: &[String]) -> Option<BTreeMap<String, bool>> {
    let mut t = BTreeMap::new();
    for l in labels {
        t.insert(l.clone(), truth_of_label(l)?);
    }
    let (mut yes, mut no) = (false, false);
    for v in t.values() {
        if *v { yes = true } else { no = true }
    }
    (yes && no).then_some(t)
}

/// Everything needed to publish one field, decided once from the field as the
/// device describes it and its mapping entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// From the device's number to the published one.
    pub conv: Conversion,
    /// Unit metadata for the path; `None` for a boolean, label or text.
    pub unit: Option<&'static str>,
    /// Enum label → boolean, when the field publishes as a boolean.
    pub truth: BTreeMap<String, bool>,
    /// Negate the boolean.
    pub invert: bool,
    /// Worth saying once at startup, though the field still publishes.
    pub warning: Option<String>,
}

/// Why a mapping entry cannot be published as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The device unit cannot reach the unit the leaf is documented to want.
    Units {
        /// The device unit, normalised.
        device: String,
        /// What the leaf wants.
        leaf: &'static str,
    },
    /// The leaf is a boolean, the field is an enum, and this build cannot tell
    /// which labels mean `true`: the mapping needs a truth table.
    Truth {
        /// The enum's labels.
        labels: Vec<String>,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Units { device, leaf } => {
                write!(
                    f,
                    "{device:?} cannot be converted to the {leaf} this leaf expects"
                )
            }
            Refusal::Truth { labels } => write!(
                f,
                "a boolean leaf needs a truth table for {}",
                labels.join(" / ")
            ),
        }
    }
}

/// Decide how a field publishes to its mapped path.
///
/// `device_unit` and `options` are the field as the device describes it: its
/// unit string, and its labels if it is an enum (empty otherwise). The rules:
///
/// - The published unit and conversion come from the device unit alone. A
///   spec leaf this build has never heard of, or a custom one, still gets
///   correct metadata.
/// - A leaf in [`leaf_unit`] cross-checks: a device unit that cannot reach it
///   is refused rather than published as a wrong number. A field with no unit
///   at all is taken at the leaf's word, because several MAC fields report
///   volts with an empty unit.
/// - A device unit this build cannot convert publishes as reported, with a
///   warning and no metadata.
/// - An enum onto a boolean leaf publishes `true` / `false` through a truth
///   table: the entry's own, else the conventional one for its labels, else
///   it is refused until a human supplies one.
pub fn plan(
    path: &str,
    device_unit: &str,
    options: &[String],
    entry: &FieldMapping,
) -> Result<Plan, Refusal> {
    let dev = units::normalize(device_unit);
    let leaf = leaf_unit(path);
    let si = units::to_si(device_unit);
    let boolean_leaf = leaf_is_boolean(path);

    let mut truth = entry.truth.clone();
    if boolean_leaf && !options.is_empty() {
        if truth.is_empty() {
            truth = truth_default(options).ok_or_else(|| Refusal::Truth {
                labels: options.to_vec(),
            })?;
        } else {
            // A table the user started is completed from the conventional
            // meanings; a label neither covers is still theirs to decide.
            for l in options {
                if truth.keys().any(|k| k.eq_ignore_ascii_case(l)) {
                    continue;
                }
                let b = truth_of_label(l).ok_or_else(|| Refusal::Truth {
                    labels: options.to_vec(),
                })?;
                truth.insert(l.clone(), b);
            }
        }
    }
    if boolean_leaf && si.is_some() {
        return Err(Refusal::Units {
            device: dev,
            leaf: "boolean",
        });
    }

    let (conv, unit, warning) = match (leaf, si) {
        (Some(l), _) if dev.is_empty() => (Conversion::IDENTITY, Some(l), None),
        (Some(l), Some(si)) if si.unit == l => (si.conv, Some(l), None),
        (Some(l), _) => {
            return Err(Refusal::Units {
                device: dev,
                leaf: l,
            });
        }
        (None, Some(si)) => (si.conv, Some(si.unit), None),
        (None, None) if dev.is_empty() => (Conversion::IDENTITY, None, None),
        (None, None) => (
            Conversion::IDENTITY,
            None,
            Some(format!(
                "unit {dev:?} is not one this build can convert to SI; values publish as \
                 reported, without unit metadata"
            )),
        ),
    };
    let unit = if boolean_leaf || !truth.is_empty() {
        None
    } else {
        unit
    };
    Ok(Plan {
        conv,
        unit,
        truth,
        invert: entry.invert,
        warning,
    })
}

/// The Signal K *node* a path belongs to: the sub-tree that carries a device's
/// static `name` and `manufacturer` metadata.
///
/// Signal K branches nest their instance at different depths, so this is a
/// small table rather than a rule. `None` for a branch with no per-device node,
/// which simply means no static metadata is published for that path.
///
/// A device can own several nodes: a CombiMaster publishes into both
/// `electrical.inverters.<id>` and `electrical.chargers.<id>`, and both should
/// carry its name.
pub fn node_of(path: &str) -> Option<String> {
    let seg: Vec<&str> = path.split('.').collect();
    let depth = match *seg.first()? {
        // electrical.<category>.<instance>.…
        "electrical" | "tanks" => 3,
        // propulsion.<instance>.…
        "propulsion" | "sails" => 2,
        _ => return None,
    };
    if seg.len() <= depth {
        return None;
    }
    Some(seg[..depth].join("."))
}

/// The instance segment of a path: the last segment of its node.
///
/// Used to keep a device's recorded instance honest. A curated path is typed by
/// hand, so it often does not contain the instance that was proposed for the
/// device: someone maps an `INT Nav Chg` onto `electrical.chargers.nav-battery`
/// because that is what the thing charges. Recording `nav-chg` as the instance
/// would then be a lie, and the mapping editor's "apply to this article" copies
/// by substituting the instance segment — so it would substitute nothing and
/// give two devices the same node.
pub fn instance_of(path: &str) -> Option<String> {
    node_of(path)?.rsplit('.').next().map(str::to_string)
}

/// Encode a device value as the JSON a Signal K delta carries.
///
/// `conv` is the unit conversion derived from the field's unit (see
/// [`crate::units::to_si`]); `invert` negates a boolean; `truth` turns an
/// enum's label into a boolean when it is non-empty. `None` means there is
/// nothing to publish this cycle: a quiet-NaN float, a negative "no value"
/// time, an unresolved enum, a label the truth table does not list, or a value
/// kind Signal K has no representation for.
pub fn encode(
    value: &Value,
    conv: Conversion,
    invert: bool,
    truth: &BTreeMap<String, bool>,
) -> Option<serde_json::Value> {
    match value {
        Value::Float(x) if x.is_finite() => Some(serde_json::Value::from(conv.apply(*x as f64))),
        Value::Float(_) => None,
        Value::Boolean(b) => Some(serde_json::Value::Bool(b ^ invert)),
        // -1 in any component is the device's "no value" sentinel.
        Value::Time(t) if t.sec >= 0 => {
            let secs = t.days as f64 * 86_400.0
                + t.hour as f64 * 3_600.0
                + t.min as f64 * 60.0
                + t.sec as f64;
            Some(serde_json::Value::from(conv.apply(secs)))
        }
        Value::Time(_) => None,
        // An enum with a truth table is a boolean in disguise.
        Value::List { .. } | Value::Eventable { .. } if !truth.is_empty() => {
            let label = value.label()?;
            let b = truth
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(label))
                .map(|(_, v)| *v)?;
            Some(serde_json::Value::Bool(b ^ invert))
        }
        // Enum-like values publish their selected label, lowercased to match
        // Signal K's convention for mode strings.
        Value::List { .. } | Value::Eventable { .. } => value
            .label()
            .map(|s| serde_json::Value::String(s.to_ascii_lowercase())),
        Value::Text { text, .. } if !text.is_empty() => {
            Some(serde_json::Value::String(text.clone()))
        }
        Value::Text { .. } | Value::Date(_) | Value::DeviceRef { .. } | Value::Invalid => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::conversion;
    use masterbus::value::Time;

    const NO_TRUTH: &BTreeMap<String, bool> = &BTreeMap::new();
    fn conv(dev: &str, path: &str) -> Conversion {
        conversion(dev, leaf_unit(path)).expect("convertible")
    }

    #[test]
    fn floats_are_converted_on_the_way_out() {
        let p = "electrical.batteries.x.temperature";
        let v = encode(&Value::Float(20.0), conv("\u{b0}C", p), false, NO_TRUTH).unwrap();
        assert!((v.as_f64().unwrap() - 293.15).abs() < 1e-9);
    }

    #[test]
    fn a_quiet_nan_publishes_nothing() {
        let p = "electrical.batteries.x.voltage";
        assert!(encode(&Value::Float(f32::NAN), conv("V", p), false, NO_TRUTH).is_none());
    }

    #[test]
    fn booleans_honour_invert() {
        let c = Conversion::IDENTITY;
        assert_eq!(
            encode(&Value::Boolean(true), c, false, NO_TRUTH).unwrap(),
            true
        );
        assert_eq!(
            encode(&Value::Boolean(true), c, true, NO_TRUTH).unwrap(),
            false
        );
    }

    #[test]
    fn time_becomes_seconds_and_the_sentinel_publishes_nothing() {
        let p = "electrical.batteries.x.capacity.timeRemaining";
        let t = Time {
            sec: 30,
            min: 2,
            hour: 1,
            days: 1,
        };
        let v = encode(&Value::Time(t), conv("", p), false, NO_TRUTH).unwrap();
        assert_eq!(v.as_f64().unwrap(), 86_400.0 + 3_600.0 + 120.0 + 30.0);
        let none = Time {
            sec: -1,
            min: 0,
            hour: 0,
            days: 0,
        };
        assert!(encode(&Value::Time(none), conv("", p), false, NO_TRUTH).is_none());
    }

    #[test]
    fn enums_publish_their_lowercased_label() {
        let v = Value::List {
            index: 2,
            options: vec!["Off".into(), "Bulk".into(), "Float".into()],
        };
        assert_eq!(
            encode(&v, Conversion::IDENTITY, false, NO_TRUTH).unwrap(),
            serde_json::Value::String("float".into())
        );
        // An unresolved enum has nothing to say.
        let bare = Value::List {
            index: 2,
            options: vec![],
        };
        assert!(encode(&bare, Conversion::IDENTITY, false, NO_TRUTH).is_none());
    }

    #[test]
    fn the_instance_is_the_nodes_last_segment() {
        assert_eq!(
            instance_of("electrical.chargers.nav-battery.voltage").as_deref(),
            Some("nav-battery")
        );
        assert_eq!(
            instance_of("electrical.inverters.mass-sine.enabled").as_deref(),
            Some("mass-sine")
        );
        assert_eq!(
            instance_of("propulsion.port.revolutions").as_deref(),
            Some("port")
        );
        // A branch with no per-device node has no instance to report.
        assert_eq!(instance_of("environment.outside.temperature"), None);
    }

    #[test]
    fn nodes_follow_the_branch_shape() {
        assert_eq!(
            node_of("electrical.batteries.house.capacity.stateOfCharge").as_deref(),
            Some("electrical.batteries.house")
        );
        assert_eq!(
            node_of("electrical.chargers.x.acin.voltage").as_deref(),
            Some("electrical.chargers.x")
        );
        assert_eq!(
            node_of("propulsion.port.revolutions").as_deref(),
            Some("propulsion.port")
        );
        // Not deep enough to name an instance, and an unknown branch.
        assert_eq!(node_of("electrical.batteries.house"), None);
        assert_eq!(node_of("environment.outside.temperature"), None);
    }

    #[test]
    fn leaf_unit_reads_the_last_segment_only() {
        assert_eq!(leaf_unit("electrical.batteries.house.voltage"), Some("V"));
        assert_eq!(leaf_unit("voltage"), Some("V"));
        assert_eq!(
            leaf_unit("electrical.chargers.x.acin.currentLimit"),
            Some("A")
        );
    }

    #[test]
    fn unitless_and_unknown_leaves_are_none() {
        assert_eq!(leaf_unit("electrical.chargers.x.chargingMode"), None);
        assert_eq!(leaf_unit("electrical.chargers.x.enabled"), None);
        assert_eq!(leaf_unit(""), None);
    }

    fn entry(path: &str) -> FieldMapping {
        FieldMapping {
            path: path.into(),
            ..Default::default()
        }
    }

    fn scalar(path: &str, unit: &str) -> Result<Plan, Refusal> {
        plan(path, unit, &[], &entry(path))
    }

    /// The point of the redesign: a spec leaf this build never tabulated
    /// still carries correct metadata, because it comes from the device.
    #[test]
    fn an_unknown_leaf_gets_its_unit_from_the_device() {
        let p = scalar("electrical.solar.sp.somethingTheSpecAddedLater", "V").unwrap();
        assert_eq!(p.unit, Some("V"));
        assert!(p.conv.is_identity());
        assert!(p.warning.is_none());

        let p = scalar("electrical.solar.sp.totalEnergy", "kWh").unwrap();
        assert_eq!(p.unit, Some("J"));
        assert!((p.conv.apply(2.0) - 7_200_000.0).abs() < 1e-6);
    }

    #[test]
    fn a_known_leaf_cross_checks_the_device_unit() {
        let p = scalar("electrical.batteries.x.temperature", "\u{b0}C").unwrap();
        assert_eq!(p.unit, Some("K"));
        assert!((p.conv.apply(20.0) - 293.15).abs() < 1e-9);
        // Amps into a voltage leaf is a mistake, not a custom choice.
        assert_eq!(
            scalar("electrical.batteries.x.voltage", "A"),
            Err(Refusal::Units {
                device: "A".into(),
                leaf: "V"
            })
        );
        // Energy into a power leaf, the case from issue #12.
        assert!(matches!(
            scalar("electrical.solar.x.power", "kWh"),
            Err(Refusal::Units { leaf: "W", .. })
        ));
    }

    #[test]
    fn a_field_with_no_unit_is_taken_at_the_leafs_word() {
        let p = scalar("electrical.chargers.x.voltage", "").unwrap();
        assert_eq!(p.unit, Some("V"));
        assert!(p.conv.is_identity());
        // And with no leaf either, there is simply nothing to say.
        let p = scalar("electrical.chargers.x.chargingMode", "").unwrap();
        assert_eq!(p.unit, None);
        assert!(p.warning.is_none());
    }

    #[test]
    fn an_unconvertible_device_unit_publishes_raw_with_a_warning() {
        let p = scalar("electrical.x.y.flow", "l/h").unwrap();
        assert_eq!(p.unit, None);
        assert!(p.conv.is_identity());
        assert!(p.warning.unwrap().contains("l/h"));
    }

    fn labels(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn known_label_pairs_get_a_truth_table_without_asking() {
        let p = plan(
            "electrical.switches.x.state",
            "",
            &labels(&["Standby", "Activated"]),
            &entry("electrical.switches.x.state"),
        )
        .unwrap();
        assert!(!p.truth["Standby"]);
        assert!(p.truth["Activated"]);
        assert_eq!(p.unit, None);
        assert!(truth_default(&labels(&["Off", "On"])).is_some());
        assert!(truth_default(&labels(&["Standby", "On"])).is_some());
    }

    #[test]
    fn an_unclassifiable_label_asks_for_a_truth_table() {
        let l = labels(&["Standby", "On", "Alarm"]);
        assert_eq!(truth_default(&l), None);
        assert_eq!(
            plan(
                "electrical.inverters.x.enabled",
                "",
                &l,
                &entry("x.enabled")
            ),
            Err(Refusal::Truth { labels: l.clone() })
        );
        // A supplied table settles it.
        let mut e = entry("electrical.inverters.x.enabled");
        e.truth = [("Standby", false), ("On", true), ("Alarm", false)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let p = plan("electrical.inverters.x.enabled", "", &l, &e).unwrap();
        assert_eq!(p.truth.len(), 3);
    }

    /// A half-filled table (the editor was backed out of) is completed from
    /// the conventional labels where it can be, and refused where it cannot.
    #[test]
    fn a_partial_table_is_completed_or_refused() {
        let l = labels(&["Standby", "On", "Alarm"]);
        let mut e = entry("electrical.inverters.x.enabled");
        e.truth = [("Alarm".to_string(), false)].into();
        let p = plan("electrical.inverters.x.enabled", "", &l, &e).unwrap();
        assert_eq!(p.truth.len(), 3);
        assert!(p.truth["On"]);
        // Two unconventional labels, one decided: still incomplete.
        let l = labels(&["Alarm", "Fault"]);
        assert!(matches!(
            plan("electrical.inverters.x.enabled", "", &l, &e),
            Err(Refusal::Truth { .. })
        ));
    }

    #[test]
    fn a_table_with_only_one_value_is_not_a_table() {
        assert_eq!(truth_default(&labels(&["On", "Yes"])), None);
    }

    #[test]
    fn a_scalar_cannot_be_published_to_a_boolean_leaf() {
        assert!(matches!(
            scalar("electrical.inverters.x.enabled", "V"),
            Err(Refusal::Units {
                leaf: "boolean",
                ..
            })
        ));
        // A real boolean field (no unit, no labels) is fine.
        let p = scalar("electrical.inverters.x.enabled", "").unwrap();
        assert!(p.truth.is_empty());
        assert_eq!(p.unit, None);
    }

    #[test]
    fn an_enum_with_a_truth_table_publishes_booleans() {
        let v = Value::List {
            index: 1,
            options: labels(&["Standby", "On"]),
        };
        let truth: BTreeMap<String, bool> = truth_default(&labels(&["Standby", "On"])).unwrap();
        assert_eq!(
            encode(&v, Conversion::IDENTITY, false, &truth).unwrap(),
            true
        );
        assert_eq!(
            encode(&v, Conversion::IDENTITY, true, &truth).unwrap(),
            false
        );
        // Lookup is case-insensitive, and a label the table lacks says nothing.
        let lower: BTreeMap<String, bool> = [("standby".to_string(), false)].into();
        let off = Value::List {
            index: 0,
            options: labels(&["Standby", "On"]),
        };
        assert_eq!(
            encode(&off, Conversion::IDENTITY, false, &lower).unwrap(),
            false
        );
        assert!(encode(&v, Conversion::IDENTITY, false, &lower).is_none());
    }

    #[test]
    fn boolean_leaves_are_the_spec_ones() {
        assert!(leaf_is_boolean("electrical.chargers.x.enabled"));
        assert!(leaf_is_boolean("electrical.switches.x.state"));
        assert!(!leaf_is_boolean("notifications.x.state"));
        assert!(!leaf_is_boolean("electrical.chargers.x.chargingMode"));
    }

    #[test]
    fn the_spec_leaves_from_the_field_reports_are_known() {
        for (leaf, unit) in [
            ("panelVoltage", "V"),
            ("panelCurrent", "A"),
            ("loadCurrent", "A"),
            ("lineNeutralVoltage", "V"),
            ("realPower", "W"),
            ("yieldToday", "J"),
            ("yieldTotal", "J"),
        ] {
            assert_eq!(
                leaf_unit(&format!("electrical.x.y.{leaf}")),
                Some(unit),
                "{leaf}"
            );
        }
    }
}
