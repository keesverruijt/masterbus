//! The CAN classes of a device's four tabs (PROTOCOL.md §2.1).
//!
//! Every device has the same four tabs: Monitoring, Alarm, History and
//! Configuration. Each tab has three kinds of message: its **values**
//! (DATA), the per-field **properties** (name, unit, range, …) and its
//! **groups**. Each kind has a request, an answer and a "not available"
//! answer, each on its own CAN class and on either the device's real
//! address or the address with bit 23 set ([`HIGH`]). The table is
//! MasterAdjust's (`mvMasterbusCanPacketTypes.pas`), where a message type is
//! `class << 1 | bit 23`.

use crate::model::Menu;

/// Address bit 23, which half of the message types are sent to or from.
pub const HIGH: u32 = 0x80_0000;

/// One message type: a CAN class, on the real address or on [`HIGH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wire {
    /// The CAN class.
    pub class: u8,
    /// Whether the address has bit 23 set.
    pub high: bool,
}

impl Wire {
    const fn real(class: u8) -> Wire {
        Wire { class, high: false }
    }

    const fn high(class: u8) -> Wire {
        Wire { class, high: true }
    }

    /// The 29-bit CAN id of this message type for a device.
    pub fn can_id(self, device_addr: u32) -> u32 {
        let addr = if self.high {
            device_addr | HIGH
        } else {
            device_addr & !HIGH
        };
        ((self.class as u32) << 24) | (addr & 0x00_FF_FF_FF)
    }

    /// Whether a frame with this class and address is this message type.
    pub fn is(self, class: u8, device_addr: u32) -> bool {
        self.class == class && (device_addr & HIGH != 0) == self.high
    }
}

/// The request, answer and "not available" answer of one kind of message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exchange {
    /// Sent to the device.
    pub req: Wire,
    /// The device's answer.
    pub info: Wire,
    /// The device's answer when it has nothing for the request.
    pub na: Wire,
}

/// The three kinds of message a tab has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Values: read `[idx_lo, idx_hi]`, write `[idx_lo, idx_hi, f32]`, answer
    /// `[idx_lo, idx_hi, f32]`.
    Data,
    /// Per-field properties: `[op, idx_lo, idx_hi]` (§6).
    Property,
    /// Groups: `[op, group, 0x00]`, field ids `[0x03, group, 0x00, i]` (§5).
    Group,
}

const fn x(req: Wire, info: Wire, na: Wire) -> Exchange {
    Exchange { req, info, na }
}

/// The message types of `kind` on `menu`'s tab.
pub const fn exchange(menu: Menu, kind: Kind) -> Exchange {
    use Wire as W;
    match (menu, kind) {
        (Menu::Monitoring, Kind::Data) => x(W::real(0x18), W::real(0x08), W::real(0x10)),
        (Menu::Monitoring, Kind::Property) => x(W::high(0x18), W::high(0x08), W::high(0x10)),
        (Menu::Monitoring, Kind::Group) => x(W::real(0x19), W::real(0x09), W::real(0x11)),
        (Menu::Alarm, Kind::Data) => x(W::real(0x03), W::real(0x02), W::high(0x02)),
        (Menu::Alarm, Kind::Property) => x(W::high(0x19), W::high(0x09), W::high(0x11)),
        (Menu::Alarm, Kind::Group) => x(W::real(0x1A), W::real(0x0A), W::real(0x12)),
        (Menu::History, Kind::Data) => x(W::high(0x1D), W::high(0x0D), W::high(0x15)),
        (Menu::History, Kind::Property) => x(W::high(0x1A), W::high(0x0A), W::high(0x12)),
        (Menu::History, Kind::Group) => x(W::real(0x1B), W::real(0x0B), W::real(0x13)),
        (Menu::Configuration, Kind::Data) => x(W::high(0x1B), W::high(0x0B), W::high(0x13)),
        (Menu::Configuration, Kind::Property) => x(W::real(0x1C), W::real(0x0C), W::real(0x14)),
        (Menu::Configuration, Kind::Group) => x(W::high(0x1C), W::high(0x0C), W::high(0x14)),
    }
}

/// What a frame from a device answers: the tab, the kind, and whether it is
/// a "not available" answer. `None` for a frame that is no tab's answer.
pub fn answer(class: u8, device_addr: u32) -> Option<(Menu, Kind, bool)> {
    for menu in Menu::ALL {
        for kind in [Kind::Data, Kind::Property, Kind::Group] {
            let e = exchange(menu, kind);
            if e.info.is(class, device_addr) {
                return Some((menu, kind, false));
            }
            if e.na.is(class, device_addr) {
                return Some((menu, kind, true));
            }
        }
    }
    None
}

/// Waiter key of a group answer (`op` `0x07` or `0x28`). `addr` is the
/// device's real address.
pub fn group_key(menu: Menu, addr: u32, op: u8, group: u8) -> String {
    format!("group:{}:{addr:06X}:{op:02X}:{group}", menu.tag())
}

/// Waiter key of the answer giving the field id at position `i` of a group.
pub fn group_field_key(menu: Menu, addr: u32, group: u8, i: u8) -> String {
    format!("group:{}:{addr:06X}:03:{group}:{i}", menu.tag())
}

/// Waiter key of a property answer.
pub fn property_key(menu: Menu, addr: u32, op: u8, index: u8) -> String {
    format!("prop:{}:{addr:06X}:{op:02X}:{index}", menu.tag())
}

/// Waiter key of an option-label answer.
pub fn option_key(menu: Menu, addr: u32, index: u8, opt: u8) -> String {
    format!("prop:{}:{addr:06X}:26:{index}:{opt}", menu.tag())
}

/// Waiter key of a tab's group or property answer, from the frame. `None`
/// for any other frame, values included: those go to the value layer.
pub fn waiter_key(class: u8, device_addr: u32, data: &[u8]) -> Option<String> {
    let (menu, kind, _) = answer(class, device_addr)?;
    let addr = device_addr & !HIGH;
    if data.len() < 2 {
        return None;
    }
    match kind {
        Kind::Data => None,
        Kind::Property if data[0] == super::meta_op::OPTION && data.len() >= 4 => {
            Some(option_key(menu, addr, data[1], data[3]))
        }
        Kind::Property => Some(property_key(menu, addr, data[0], data[1])),
        Kind::Group if data[0] == 0x03 => {
            (data.len() >= 4).then(|| group_field_key(menu, addr, data[1], data[3]))
        }
        Kind::Group => Some(group_key(menu, addr, data[0], data[1])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two message types share a class and address half, so an answer
    /// names its tab and kind.
    #[test]
    fn every_answer_is_one_tab_and_kind() {
        let mut seen = Vec::new();
        for menu in Menu::ALL {
            for kind in [Kind::Data, Kind::Property, Kind::Group] {
                let e = exchange(menu, kind);
                for w in [e.info, e.na] {
                    assert!(!seen.contains(&w), "{w:?} twice");
                    seen.push(w);
                    let addr = if w.high { 0x2A42F5 | HIGH } else { 0x2A42F5 };
                    assert_eq!(answer(w.class, addr), Some((menu, kind, w == e.na)));
                }
            }
        }
    }

    /// The classes MasterAdjust uses on a ChargeMaster Plus (#6).
    #[test]
    fn the_classes_match_masteradjusts_trace() {
        let e = exchange(Menu::Configuration, Kind::Group);
        assert_eq!(e.req.can_id(0x2A42F5), 0x1CAA42F5);
        let e = exchange(Menu::Configuration, Kind::Data);
        assert_eq!(e.req.can_id(0x2A42F5), 0x1BAA42F5);
        let e = exchange(Menu::Alarm, Kind::Data);
        assert_eq!(e.req.can_id(0x2A42F5), 0x032A42F5);
        let e = exchange(Menu::History, Kind::Data);
        assert_eq!(e.req.can_id(0x2A42F5), 0x1DAA42F5);
    }
}
