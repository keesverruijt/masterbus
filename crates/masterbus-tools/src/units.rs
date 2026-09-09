//! Device unit → Signal K SI unit conversion, *derived* rather than stored.
//!
//! A hand-curated mapping file says which Signal K path a field publishes to.
//! It deliberately does not say how to scale the number, because a stored
//! factor is one more thing a human can get wrong, and because the pair of
//! units already determines the answer: a field reporting `°C` into a leaf
//! that wants `K` can only mean one thing.
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

/// The conversion from a device unit to a Signal K leaf unit, or `None` when
/// the pair is not convertible.
///
/// `None` is the signal to warn about a mapping entry rather than to publish a
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
    let dev = normalize(device_unit);
    let scale = |s: f64| {
        Some(Conversion {
            scale: s,
            offset: 0.0,
        })
    };
    match (dev.as_str(), sk) {
        // Same unit on both sides.
        (d, s) if d == s => Some(Conversion::IDENTITY),
        // A device that reports no unit at all is taken at its word: several
        // MAC monitoring fields report volts and amps with an empty unit.
        ("", _) => Some(Conversion::IDENTITY),
        ("\u{b0}C", "K") => Some(Conversion {
            scale: 1.0,
            offset: 273.15,
        }),
        ("%", "ratio") => scale(0.01),
        ("Ah", "C") => scale(3600.0),
        ("rpm", "Hz") => scale(1.0 / 60.0),
        ("kWh", "J") => scale(3_600_000.0),
        _ => None,
    }
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
    }

    #[test]
    fn normalize_folds_the_spellings_seen_in_the_field() {
        assert_eq!(normalize("RPM"), "rpm");
        assert_eq!(normalize(" "), "");
        assert_eq!(normalize(" V "), "V");
        assert_eq!(normalize("\u{b0}C"), "\u{b0}C");
    }
}
