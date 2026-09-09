//! The Signal K side of the mapping: what a path *means*, independent of any
//! particular MasterBus device.

use masterbus::Value;

use crate::units::Conversion;

/// The SI unit Signal K expects at a given path, keyed on the path's last
/// segment (its "leaf"). `None` for a leaf that carries no unit — a string
/// state such as `chargingMode`, a boolean such as `enabled`, or a leaf this
/// table does not know.
///
/// This is what makes stored conversion factors unnecessary: given a field's
/// unit as the device reports it and the unit the target leaf wants,
/// [`crate::units::conversion`] derives the arithmetic.
pub fn leaf_unit(path: &str) -> Option<&'static str> {
    Some(match path.rsplit('.').next().unwrap_or("") {
        "stateOfCharge" => "ratio",
        "timeRemaining" => "s",
        "dischargeSinceFull" => "C",
        "temperature" => "K",
        "voltage" | "voltageSense" => "V",
        "current" | "currentLimit" => "A",
        "power" => "W",
        "frequency" | "revolutions" => "Hz",
        _ => return None,
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

/// Encode a device value as the JSON a Signal K delta carries.
///
/// `conv` is the unit conversion derived from the field's unit and the target
/// leaf (see [`crate::units::conversion`]); `invert` negates a boolean. `None`
/// means there is nothing to publish this cycle: a quiet-NaN float, a negative
/// "no value" time, an unresolved enum, or a value kind Signal K has no
/// representation for.
pub fn encode(value: &Value, conv: Conversion, invert: bool) -> Option<serde_json::Value> {
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

    fn conv(dev: &str, path: &str) -> Conversion {
        conversion(dev, leaf_unit(path)).expect("convertible")
    }

    #[test]
    fn floats_are_converted_on_the_way_out() {
        let p = "electrical.batteries.x.temperature";
        let v = encode(&Value::Float(20.0), conv("\u{b0}C", p), false).unwrap();
        assert!((v.as_f64().unwrap() - 293.15).abs() < 1e-9);
    }

    #[test]
    fn a_quiet_nan_publishes_nothing() {
        let p = "electrical.batteries.x.voltage";
        assert!(encode(&Value::Float(f32::NAN), conv("V", p), false).is_none());
    }

    #[test]
    fn booleans_honour_invert() {
        let c = Conversion::IDENTITY;
        assert_eq!(encode(&Value::Boolean(true), c, false).unwrap(), true);
        assert_eq!(encode(&Value::Boolean(true), c, true).unwrap(), false);
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
        let v = encode(&Value::Time(t), conv("", p), false).unwrap();
        assert_eq!(v.as_f64().unwrap(), 86_400.0 + 3_600.0 + 120.0 + 30.0);
        let none = Time {
            sec: -1,
            min: 0,
            hour: 0,
            days: 0,
        };
        assert!(encode(&Value::Time(none), conv("", p), false).is_none());
    }

    #[test]
    fn enums_publish_their_lowercased_label() {
        let v = Value::List {
            index: 2,
            options: vec!["Off".into(), "Bulk".into(), "Float".into()],
        };
        assert_eq!(
            encode(&v, Conversion::IDENTITY, false).unwrap(),
            serde_json::Value::String("float".into())
        );
        // An unresolved enum has nothing to say.
        let bare = Value::List {
            index: 2,
            options: vec![],
        };
        assert!(encode(&bare, Conversion::IDENTITY, false).is_none());
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
}
