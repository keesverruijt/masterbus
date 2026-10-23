//! This program's own BusID: the writer a commit write names (PROTOCOL.md §7.3).
//!
//! A real device's BusID is fixed at the factory: a 5-bit product class from
//! its firmware, and 18 bits computed from its serial and article numbers
//! (PROTOCOL.md §3a). Those 18 bits can be anything, so only the class keeps
//! us apart from real devices. [`CLASS`] is one no Mastervolt firmware or bus
//! seen so far uses, and no firmware treats specially. The low 18 bits are
//! random, so two installations on one bus differ; `config.ini` keeps them.

pub use crate::protocol::{addr_for_bus_id, bus_id_for_addr};

/// The product class of our own BusID.
pub const CLASS: u8 = 0x1C;

/// Whether `bus_id` is a BusID at all: bits 18..23 are unused, and the class
/// is 5 bits.
pub fn is_valid(bus_id: u32) -> bool {
    bus_id & 0xE0FC_0000 == 0
}

/// A fresh BusID in [`CLASS`] with random, non-zero low bits.
pub fn generate() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.write_u32(std::process::id());
    let low = (h.finish() as u32 & 0x3FFFF).max(1);
    (u32::from(CLASS) << 24) | low
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_id_is_ours_and_valid() {
        for _ in 0..100 {
            let id = generate();
            assert!(is_valid(id), "{id:08X}");
            assert_eq!(id >> 24, u32::from(CLASS));
            assert_ne!(id & 0x3FFFF, 0);
        }
    }

    #[test]
    fn real_bus_ids_are_valid_and_junk_is_not() {
        assert!(is_valid(0x140294F9));
        assert!(is_valid(0x1B03FFFF));
        assert!(!is_valid(0x14FC0000));
        assert!(!is_valid(0x20000001));
    }
}
