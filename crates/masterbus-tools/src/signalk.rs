//! The Signal K side of the mapping: what a path *means*, independent of any
//! particular MasterBus device.

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

#[cfg(test)]
mod tests {
    use super::*;

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
