//! Device unit → Signal K SI unit conversion, *derived* rather than stored.
//!
//! A hand-curated mapping file says which Signal K path a field publishes to.
//! It deliberately does not say how to scale the number, because a stored
//! factor is one more thing a human can get wrong, and because the device's
//! own unit already determines the answer: Signal K uses exactly one SI unit
//! per quantity, so a field reporting `°C` can only ever become kelvin,
//! whatever path it is published to.
//!
//! That is the whole idea of [`to_si`]: the Signal K unit and the arithmetic
//! follow from the device unit alone. The name of the target leaf is not
//! needed for it, which is what lets a mapping point at a spec path this build
//! has never heard of, or at a custom one, and still carry correct unit
//! metadata. The leaf table in [`crate::signalk`] only *cross-checks*: it
//! catches an ampere field pointed at a `voltage` leaf.
//!
//! Device units are messy in the field. Observed on a single 28-device bus:
//! `RPM` and `rpm` on different devices, a unit that is one space character,
//! and empty units on a charger that does report numbers. [`normalize`]
//! absorbs that so the table below stays small.

/// A linear conversion from a device value to its SI equivalent:
/// `si = raw * scale + offset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conversion {
    /// Multiplicative factor.
    pub scale: f64,
    /// Additive offset, applied after scaling.
    pub offset: f64,
}

impl Conversion {
    /// The no-op conversion, for when device and Signal K agree on the unit.
    pub const IDENTITY: Conversion = Conversion {
        scale: 1.0,
        offset: 0.0,
    };

    /// Apply to a raw device value.
    pub fn apply(self, raw: f64) -> f64 {
        raw * self.scale + self.offset
    }

    /// Whether this conversion leaves the value untouched.
    pub fn is_identity(self) -> bool {
        self.scale == 1.0 && self.offset == 0.0
    }

    /// A human-readable form of the arithmetic, for the editor.
    pub fn describe(self) -> String {
        if self.is_identity() {
            "unchanged".into()
        } else {
            format!(
                "×{} {}{}",
                self.scale,
                if self.offset >= 0.0 { "+" } else { "−" },
                self.offset.abs()
            )
        }
    }
}

/// The SI form of a device unit: what Signal K calls it, and how to get there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Si {
    /// The unit Signal K publishes, as it appears in `meta.units`.
    pub unit: &'static str,
    /// From the device's number to that unit.
    pub conv: Conversion,
}

/// Canonical form of a unit string as a device reports it: trimmed, and
/// case-folded for the units that are observed spelled both ways. Returns an
/// empty string for a unit that is absent or whitespace only.
pub fn normalize(unit: &str) -> String {
    let t = unit.trim();
    match t {
        "RPM" | "rpm" => "rpm".to_string(),
        // Celsius is written with the degree sign on every device seen so far,
        // but accept the bare form so a future firmware does not silently drop
        // temperatures.
        "C" => "\u{b0}C".to_string(),
        other => other.to_string(),
    }
}

/// The Signal K unit and conversion a device unit implies, from the device
/// unit alone.
///
/// `None` for an empty unit (nothing to say) and for a unit this table does
/// not know, which is the one case worth a warning: the number will publish
/// as the device reports it, with no unit metadata.
pub fn to_si(device_unit: &str) -> Option<Si> {
    let dev = normalize(device_unit);
    let same = |u: &'static str| {
        Some(Si {
            unit: u,
            conv: Conversion::IDENTITY,
        })
    };
    let scaled = |u: &'static str, s: f64| {
        Some(Si {
            unit: u,
            conv: Conversion {
                scale: s,
                offset: 0.0,
            },
        })
    };
    match dev.as_str() {
        "" => None,
        "V" => same("V"),
        "A" => same("A"),
        "W" => same("W"),
        "Hz" => same("Hz"),
        "s" => same("s"),
        "K" => same("K"),
        "ratio" => same("ratio"),
        "\u{b0}C" => Some(Si {
            unit: "K",
            conv: Conversion {
                scale: 1.0,
                offset: 273.15,
            },
        }),
        "%" => scaled("ratio", 0.01),
        "Ah" => scaled("C", 3600.0),
        "rpm" => scaled("Hz", 1.0 / 60.0),
        "kWh" => scaled("J", 3_600_000.0),
        "Wh" => scaled("J", 3600.0),
        "kW" => scaled("W", 1000.0),
        "mV" => scaled("V", 0.001),
        "mA" => scaled("A", 0.001),
        "min" => scaled("s", 60.0),
        "h" => scaled("s", 3600.0),
        _ => None,
    }
}

/// The conversion from a device unit to a Signal K leaf unit, or `None` when
/// the pair is not convertible.
///
/// `None` is the signal to refuse a mapping entry rather than to publish a
/// wrong number: it means a human pointed, say, an ampere field at a leaf that
/// wants kelvin.
///
/// A Signal K leaf with no unit (`sk` is `None`) accepts anything, because the
/// value is not a scalar being converted: an enum publishes its label and a
/// boolean publishes itself.
pub fn conversion(device_unit: &str, sk_unit: Option<&str>) -> Option<Conversion> {
    let Some(sk) = sk_unit else {
        return Some(Conversion::IDENTITY);
    };
    // A device that reports no unit at all is taken at its word: several MAC
    // monitoring fields report volts and amps with an empty unit.
    if normalize(device_unit).is_empty() {
        return Some(Conversion::IDENTITY);
    }
    let si = to_si(device_unit)?;
    (si.unit == sk).then_some(si.conv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(dev: &str, sk: &str) -> Conversion {
        conversion(dev, Some(sk)).expect("expected a convertible pair")
    }

    #[test]
    fn celsius_becomes_kelvin() {
        assert!((conv("\u{b0}C", "K").apply(20.0) - 293.15).abs() < 1e-9);
    }

    #[test]
    fn percent_becomes_ratio() {
        assert!((conv("%", "ratio").apply(87.0) - 0.87).abs() < 1e-9);
    }

    #[test]
    fn amp_hours_become_coulombs() {
        assert_eq!(conv("Ah", "C").apply(-32.0), -115_200.0);
    }

    #[test]
    fn rpm_becomes_hertz_whatever_its_case() {
        assert!((conv("rpm", "Hz").apply(1800.0) - 30.0).abs() < 1e-9);
        assert!((conv("RPM", "Hz").apply(1800.0) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn matching_units_are_identity() {
        assert!(conv("V", "V").is_identity());
        assert!(conv("A", "A").is_identity());
    }

    #[test]
    fn a_missing_device_unit_is_taken_at_its_word() {
        // Several MAC monitoring fields report volts with an empty unit.
        assert!(conv("", "V").is_identity());
        assert!(conv("  ", "V").is_identity());
    }

    #[test]
    fn a_unitless_leaf_accepts_anything() {
        // chargingMode publishes an enum label; there is nothing to scale.
        assert!(conversion("A", None).unwrap().is_identity());
    }

    #[test]
    fn mismatched_units_refuse_rather_than_guess() {
        assert_eq!(conversion("A", Some("K")), None);
        assert_eq!(conversion("V", Some("Hz")), None);
        // Energy is not power, however tempting `.power` looks.
        assert_eq!(conversion("kWh", Some("W")), None);
    }

    /// The point of the redesign: the Signal K unit follows from the device
    /// unit alone, so a leaf nobody has tabulated still gets correct metadata.
    #[test]
    fn the_si_unit_is_known_without_a_leaf() {
        assert_eq!(to_si("V").unwrap().unit, "V");
        assert_eq!(to_si("\u{b0}C").unwrap().unit, "K");
        assert_eq!(to_si("kWh").unwrap().unit, "J");
        assert!((to_si("kWh").unwrap().conv.apply(1.5) - 5_400_000.0).abs() < 1e-6);
        assert_eq!(to_si("RPM").unwrap().unit, "Hz");
        assert_eq!(to_si(" % ").unwrap().unit, "ratio");
    }

    #[test]
    fn an_empty_or_unknown_unit_has_no_si_form() {
        assert_eq!(to_si(""), None);
        assert_eq!(to_si(" "), None);
        assert_eq!(to_si("furlongs"), None);
    }

    #[test]
    fn conversions_describe_themselves() {
        assert_eq!(Conversion::IDENTITY.describe(), "unchanged");
        assert_eq!(to_si("\u{b0}C").unwrap().conv.describe(), "×1 +273.15");
        assert_eq!(to_si("%").unwrap().conv.describe(), "×0.01 +0");
    }

    #[test]
    fn normalize_folds_the_spellings_seen_in_the_field() {
        assert_eq!(normalize("RPM"), "rpm");
        assert_eq!(normalize(" "), "");
        assert_eq!(normalize(" V "), "V");
        assert_eq!(normalize("\u{b0}C"), "\u{b0}C");
    }
}
