//! Shared in-memory state: device liveness, discovered schema, value cache.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use crate::model::{
    AccessLevel, DeviceIdentity, DeviceSchema, DeviceStatus, FieldId, FieldInfo, GroupInfo, Menu,
};
use crate::value::Value;

/// A cached field value with its observation time and a dirty flag.
#[derive(Debug, Clone)]
pub struct CachedValue {
    /// The value last seen on the bus.
    pub value: Value,
    /// When it was observed.
    pub at: Instant,
    /// Set after a write; forces a re-poll before the value is trusted.
    pub outdated: bool,
}

/// Per-device runtime state.
pub struct DeviceEntry {
    /// 24-bit CAN device address (also the public device id).
    pub addr: u32,
    /// When the device was first heard — drives the discovery settle window.
    pub first_seen: Instant,
    /// Last time a broadcast (or any frame) was heard — drives liveness.
    pub last_seen: Instant,
    /// Device-family type code from the broadcast.
    pub type_code: u8,
    /// Basic firmware hint from the broadcast (u16 LE).
    pub fw_hint: u16,
    /// Identity (cheap discovery), once available.
    pub identity: Option<DeviceIdentity>,
    /// The device's last-known access level (from a `0x08 0x19` read or a
    /// successful login/logout). Used to key the schema cache: writability
    /// flips per level, so a schema discovered at one level can't be served
    /// at another. `None` until we've queried the level once.
    pub access_level: Option<AccessLevel>,
    /// Schema discovered so far; `groups` accumulates menu-by-menu as menus are
    /// lazily discovered. `None` until at least the identity is known.
    pub schema: Option<DeviceSchema>,
    /// Which menus have been discovered into `schema.groups`.
    pub menus: HashSet<Menu>,
    /// Flat enumeration of every reachable field (probed via Btm1 metadata
    /// across the full index space `0..0x08 0x01`). Populated lazily, cached
    /// in memory until a login/logout invalidates it. `None` until probed.
    pub all_fields: Option<Vec<FieldInfo>>,
    /// Latest value per channel-aware field id.
    pub values: HashMap<FieldId, CachedValue>,
    /// Offline string-catalog binding, resolved once (after identity) via a
    /// live spot-check. `false` = not yet attempted. When attempted,
    /// `catalog_table` is `Some(table)` on a spot-check hit and `None` when the
    /// device has no usable bundled table (strings then fetched live).
    pub catalog_attempted: bool,
    /// The spot-checked static string table for this device, if any.
    pub catalog_table: Option<&'static HashMap<u16, String>>,
}

impl DeviceEntry {
    fn new(addr: u32, now: Instant) -> Self {
        DeviceEntry {
            addr,
            first_seen: now,
            last_seen: now,
            type_code: 0,
            fw_hint: 0,
            identity: None,
            access_level: None,
            schema: None,
            menus: HashSet::new(),
            all_fields: None,
            values: HashMap::new(),
            catalog_attempted: false,
            catalog_table: None,
        }
    }
}

/// Shared device table.
pub struct State {
    devices: Mutex<HashMap<u32, DeviceEntry>>,
}

impl State {
    /// Create empty state.
    pub fn new() -> Self {
        State {
            devices: Mutex::new(HashMap::new()),
        }
    }

    /// Mark a device alive (from a broadcast), updating its identity hints.
    pub fn mark_alive(&self, addr: u32, type_code: u8, fw_hint: u16) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.last_seen = now;
        e.type_code = type_code;
        e.fw_hint = fw_hint;
    }

    /// Touch last-seen for any frame from a device, creating a minimal entry
    /// on first sight. Lets the engine discover devices that don't emit the
    /// class-`0x04` broadcast in our liveness window — e.g. an EasyView that's
    /// currently being polled by another master — but are otherwise active on
    /// the bus. The `type_code` / `fw_hint` stay zero until a real broadcast
    /// arrives; UI code should treat them as "unknown" placeholders.
    pub fn touch(&self, addr: u32) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.last_seen = now;
    }

    /// Record a freshly-observed value.
    pub fn put_value(&self, addr: u32, field: FieldId, value: Value) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.values.insert(
            field,
            CachedValue {
                value,
                at: now,
                outdated: false,
            },
        );
    }

    /// Mark a field's cached value outdated (e.g. after a write).
    pub fn mark_outdated(&self, addr: u32, field: FieldId) {
        if let Some(e) = self.devices.lock().unwrap().get_mut(&addr)
            && let Some(v) = e.values.get_mut(&field)
        {
            v.outdated = true;
        }
    }

    /// Get a cached value (clone) if present.
    pub fn get_value(&self, addr: u32, field: FieldId) -> Option<CachedValue> {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .and_then(|e| e.values.get(&field).cloned())
    }

    /// Store a device's identity (cheap discovery result).
    pub fn put_identity(&self, addr: u32, identity: DeviceIdentity) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.identity = Some(identity);
    }

    /// Whether a device's identity is known (directly or via a full schema).
    pub fn has_identity(&self, addr: u32) -> bool {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .map(|e| e.identity.is_some() || e.schema.is_some())
            .unwrap_or(false)
    }

    /// Clone a device's identity if known (preferring the full schema).
    pub fn identity(&self, addr: u32) -> Option<DeviceIdentity> {
        let map = self.devices.lock().unwrap();
        let e = map.get(&addr)?;
        e.schema
            .as_ref()
            .map(|s| s.identity())
            .or_else(|| e.identity.clone())
    }

    /// Whether the offline string catalog has been resolved for this device
    /// (spot-checked once). Distinguishes "not attempted" from "attempted, no
    /// usable table".
    pub fn catalog_attempted(&self, addr: u32) -> bool {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .map(|e| e.catalog_attempted)
            .unwrap_or(false)
    }

    /// The spot-checked static string table for this device, if resolution
    /// found and confirmed one.
    pub fn catalog_table(&self, addr: u32) -> Option<&'static HashMap<u16, String>> {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .and_then(|e| e.catalog_table)
    }

    /// Record the result of a catalog resolution attempt (`Some(table)` on a
    /// spot-check hit, `None` when no usable table). Marks it attempted so we
    /// don't re-run the spot-check.
    pub fn put_catalog(&self, addr: u32, table: Option<&'static HashMap<u16, String>>) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.catalog_attempted = true;
        e.catalog_table = table;
    }

    /// Get the device's last-known access level. `None` until we've heard
    /// one (either via an explicit read or after a successful login/logout).
    pub fn access_level(&self, addr: u32) -> Option<AccessLevel> {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .and_then(|e| e.access_level)
    }

    /// Record the device's access level (after a successful read or
    /// login/logout response).
    pub fn put_access_level(&self, addr: u32, level: AccessLevel) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.access_level = Some(level);
    }

    /// Add a discovered menu's groups to a device's schema (identity must already
    /// be stored). Replaces any previously-held groups for that menu.
    pub fn put_menu(&self, addr: u32, menu: Menu, groups: Vec<GroupInfo>) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        let id = e.identity.clone().unwrap_or_else(|| DeviceIdentity {
            article: String::new(),
            serial: String::new(),
            revision: String::new(),
            name: String::new(),
            firmware: String::new(),
        });
        let schema = e
            .schema
            .get_or_insert_with(|| DeviceSchema::from_identity(id, Vec::new()));
        schema.groups.retain(|g| g.menu != menu);
        schema.groups.extend(groups);
        e.menus.insert(menu);
    }

    /// Whether a specific menu has been discovered for a device.
    pub fn has_menu(&self, addr: u32, menu: Menu) -> bool {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .map(|e| e.menus.contains(&menu))
            .unwrap_or(false)
    }

    /// Whether every menu in `menus` has been discovered for a device.
    pub fn has_menus(&self, addr: u32, menus: &[Menu]) -> bool {
        let map = self.devices.lock().unwrap();
        match map.get(&addr) {
            Some(e) => menus.iter().all(|m| e.menus.contains(m)),
            None => false,
        }
    }

    /// Whether a particular field has been discovered for a device.
    pub fn has_field(&self, addr: u32, field: FieldId) -> bool {
        self.field_info(addr, field).is_some()
    }

    /// Look up a field's info from either the menu-grouped schema (Btm1
    /// discovery) or the flat Btm3 probe list (`all_fields`). Used by the
    /// reader to decode incoming Btm3 value pushes, whose fields live in
    /// `all_fields` rather than `schema.groups`.
    pub fn field_info(&self, addr: u32, field: FieldId) -> Option<FieldInfo> {
        let map = self.devices.lock().unwrap();
        let e = map.get(&addr)?;
        if let Some(s) = e.schema.as_ref()
            && let Some(f) = s.field(field)
        {
            return Some(f.clone());
        }
        if let Some(all) = e.all_fields.as_ref() {
            return all.iter().find(|f| f.index == field).cloned();
        }
        None
    }

    /// Clone a device's schema-so-far (groups of all discovered menus).
    pub fn schema(&self, addr: u32) -> Option<DeviceSchema> {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .and_then(|e| e.schema.clone())
    }

    /// Drop the cached schema (groups + discovered-menu set) for a device.
    /// The identity is preserved; the next `tab_info` / `schema` / `field`
    /// access will re-run discovery and fetch fresh metadata attributes
    /// (writability in particular changes after an access-level login).
    pub fn forget_schema(&self, addr: u32) {
        let mut map = self.devices.lock().unwrap();
        if let Some(e) = map.get_mut(&addr) {
            e.schema = None;
            e.menus.clear();
            e.all_fields = None;
        }
    }

    /// Whether the flat field enumeration has been populated for a device.
    pub fn has_all_fields(&self, addr: u32) -> bool {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .is_some_and(|e| e.all_fields.is_some())
    }

    /// Clone the flat field enumeration for a device, if discovered.
    pub fn all_fields(&self, addr: u32) -> Option<Vec<FieldInfo>> {
        self.devices
            .lock()
            .unwrap()
            .get(&addr)
            .and_then(|e| e.all_fields.clone())
    }

    /// Store the flat field enumeration for a device.
    pub fn put_all_fields(&self, addr: u32, fields: Vec<FieldInfo>) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map
            .entry(addr)
            .or_insert_with(|| DeviceEntry::new(addr, now));
        e.all_fields = Some(fields);
    }

    /// Device ids currently considered alive (seen within `liveness`).
    pub fn alive_ids(&self, liveness: std::time::Duration) -> Vec<u32> {
        let now = Instant::now();
        let mut ids: Vec<u32> = self
            .devices
            .lock()
            .unwrap()
            .values()
            .filter(|e| now.duration_since(e.last_seen) <= liveness)
            .map(|e| e.addr)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// True if any device has ever been heard.
    pub fn any_device(&self) -> bool {
        !self.devices.lock().unwrap().is_empty()
    }

    /// When the most recently discovered device was first heard, if any.
    pub fn newest_first_seen(&self) -> Option<Instant> {
        self.devices
            .lock()
            .unwrap()
            .values()
            .map(|e| e.first_seen)
            .max()
    }

    /// Compute a device's status from liveness.
    pub fn status(&self, addr: u32, liveness: std::time::Duration) -> DeviceStatus {
        let map = self.devices.lock().unwrap();
        match map.get(&addr) {
            None => DeviceStatus::Offline,
            Some(e) => {
                if Instant::now().duration_since(e.last_seen) <= liveness {
                    DeviceStatus::On
                } else {
                    DeviceStatus::Offline
                }
            }
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::field_id;
    use crate::protocol::VisualizationType;
    use std::time::Duration;

    const A: u32 = 0x188EA2;
    const B: u32 = 0x3A3B4B;

    /// Long enough that anything touched during a test counts as alive.
    const ALIVE: Duration = Duration::from_secs(60);

    fn field(index: FieldId, name: &str) -> FieldInfo {
        FieldInfo {
            index,
            name: name.to_string(),
            unit: String::new(),
            viz_type: VisualizationType::Float,
            writeable: false,
            eventable: false,
            min: 0.0,
            max: 0.0,
            step: 0.0,
            options: Vec::new(),
        }
    }

    fn group(id: i32, menu: Menu, fields: Vec<FieldInfo>) -> GroupInfo {
        GroupInfo {
            id,
            name: format!("group {id}"),
            menu,
            fields,
        }
    }

    fn identity(name: &str) -> DeviceIdentity {
        DeviceIdentity {
            article: "44010250".into(),
            serial: "1234567".into(),
            revision: "3".into(),
            name: name.to_string(),
            firmware: "1.0".into(),
        }
    }

    #[test]
    fn a_broadcast_registers_the_device_with_its_hints() {
        let s = State::new();
        assert!(!s.any_device());
        s.mark_alive(A, 0x0B, 0x0102);
        assert!(s.any_device());
        assert_eq!(s.alive_ids(ALIVE), vec![A]);
        assert_eq!(s.status(A, ALIVE), DeviceStatus::On);

        let map = s.devices.lock().unwrap();
        let e = map.get(&A).unwrap();
        assert_eq!((e.type_code, e.fw_hint), (0x0B, 0x0102));
    }

    /// A device heard only through non-broadcast traffic still registers —
    /// that's the whole point of `touch` — but its broadcast-only hints stay
    /// at their "unknown" zero placeholders.
    #[test]
    fn touch_registers_a_device_without_inventing_hints() {
        let s = State::new();
        s.touch(A);
        assert_eq!(s.alive_ids(ALIVE), vec![A]);

        let map = s.devices.lock().unwrap();
        let e = map.get(&A).unwrap();
        assert_eq!((e.type_code, e.fw_hint), (0, 0));
    }

    #[test]
    fn a_value_round_trips_and_can_be_marked_outdated() {
        let s = State::new();
        let f = field_id::btm1(0x17);
        assert!(s.get_value(A, f).is_none());

        s.put_value(A, f, Value::Float(12.5));
        let cv = s.get_value(A, f).unwrap();
        assert_eq!(cv.value, Value::Float(12.5));
        assert!(!cv.outdated);

        s.mark_outdated(A, f);
        assert!(s.get_value(A, f).unwrap().outdated);

        // A fresh observation clears the flag again.
        s.put_value(A, f, Value::Float(13.0));
        assert!(!s.get_value(A, f).unwrap().outdated);
    }

    /// Marking an unknown device or field outdated is a no-op, not a panic:
    /// a write can race a device dropping off the bus.
    #[test]
    fn marking_an_unknown_value_outdated_is_a_no_op() {
        let s = State::new();
        s.mark_outdated(A, field_id::btm1(0x17));
        s.put_value(A, field_id::btm1(0x17), Value::Float(1.0));
        s.mark_outdated(A, field_id::btm1(0x99));
        assert!(!s.get_value(A, field_id::btm1(0x17)).unwrap().outdated);
    }

    #[test]
    fn identity_is_known_from_either_the_cheap_fetch_or_a_schema() {
        let s = State::new();
        assert!(!s.has_identity(A));
        assert!(s.identity(A).is_none());

        s.put_identity(A, identity("Combi"));
        assert!(s.has_identity(A));
        assert_eq!(s.identity(A).unwrap().name, "Combi");
    }

    /// Once a schema exists it is the authority on identity: it was discovered
    /// as a unit, so a later cheap re-fetch must not half-replace it.
    #[test]
    fn the_schema_identity_wins_over_a_later_cheap_fetch() {
        let s = State::new();
        s.put_identity(A, identity("Combi"));
        s.put_menu(
            A,
            Menu::Monitoring,
            vec![group(0, Menu::Monitoring, vec![])],
        );

        s.put_identity(A, identity("Renamed"));
        assert_eq!(s.identity(A).unwrap().name, "Combi");
    }

    #[test]
    fn a_menus_groups_are_replaced_wholesale_leaving_other_menus_alone() {
        let s = State::new();
        s.put_identity(A, identity("Combi"));
        s.put_menu(
            A,
            Menu::Monitoring,
            vec![group(
                0,
                Menu::Monitoring,
                vec![field(field_id::btm1(1), "V")],
            )],
        );
        s.put_menu(
            A,
            Menu::Configuration,
            vec![group(7, Menu::Configuration, vec![])],
        );
        assert_eq!(s.schema(A).unwrap().groups.len(), 2);

        // Re-discovering Monitoring replaces only its own groups.
        s.put_menu(
            A,
            Menu::Monitoring,
            vec![
                group(0, Menu::Monitoring, vec![]),
                group(1, Menu::Monitoring, vec![]),
            ],
        );
        let schema = s.schema(A).unwrap();
        assert_eq!(schema.menu_groups(Menu::Monitoring).count(), 2);
        assert_eq!(schema.menu_groups(Menu::Configuration).count(), 1);
    }

    /// `put_menu` before the identity is known still builds a schema — with a
    /// blank identity — rather than dropping the discovered groups.
    #[test]
    fn a_menu_discovered_before_the_identity_still_lands() {
        let s = State::new();
        s.put_menu(
            A,
            Menu::Monitoring,
            vec![group(0, Menu::Monitoring, vec![])],
        );
        assert_eq!(s.schema(A).unwrap().name, "");
        assert!(s.has_menu(A, Menu::Monitoring));
    }

    #[test]
    fn has_menus_requires_every_menu() {
        let s = State::new();
        assert!(!s.has_menu(A, Menu::Monitoring));
        assert!(!s.has_menus(A, &[Menu::Monitoring]));

        s.put_menu(A, Menu::Monitoring, vec![]);
        assert!(s.has_menu(A, Menu::Monitoring));
        assert!(s.has_menus(A, &[Menu::Monitoring]));
        assert!(!s.has_menus(A, &[Menu::Monitoring, Menu::Alarm]));

        s.put_menu(A, Menu::Alarm, vec![]);
        assert!(s.has_menus(A, &[Menu::Monitoring, Menu::Alarm]));
    }

    /// Btm3 fields live in the flat probe list, not in `schema.groups` — the
    /// reader has to find them in either place to decode a value push.
    #[test]
    fn field_info_looks_in_the_schema_and_the_flat_probe_list() {
        let s = State::new();
        let btm1 = field_id::btm1(0x17);
        let btm3 = field_id::btm3(0x30);

        s.put_identity(A, identity("Combi"));
        s.put_menu(
            A,
            Menu::Monitoring,
            vec![group(0, Menu::Monitoring, vec![field(btm1, "Voltage")])],
        );
        s.put_all_fields(A, vec![field(btm3, "ShutDown")]);

        assert_eq!(s.field_info(A, btm1).unwrap().name, "Voltage");
        assert_eq!(s.field_info(A, btm3).unwrap().name, "ShutDown");
        assert!(s.has_field(A, btm1));
        assert!(s.has_field(A, btm3));
        assert!(!s.has_field(A, field_id::btm1(0x30)));
        assert!(!s.has_field(B, btm1));
    }

    #[test]
    fn the_flat_probe_list_is_tracked_separately() {
        let s = State::new();
        assert!(!s.has_all_fields(A));
        assert!(s.all_fields(A).is_none());

        s.put_all_fields(A, vec![field(field_id::btm3(0x30), "ShutDown")]);
        assert!(s.has_all_fields(A));
        assert_eq!(s.all_fields(A).unwrap().len(), 1);
    }

    /// What a login invalidates: the schema, the discovered-menu set and the
    /// flat probe list (writability flips per access level). The identity and
    /// the cached values are level-independent and survive.
    #[test]
    fn forget_schema_drops_discovery_but_keeps_the_identity_and_values() {
        let s = State::new();
        let f = field_id::btm1(0x17);
        s.put_identity(A, identity("Combi"));
        s.put_menu(
            A,
            Menu::Monitoring,
            vec![group(0, Menu::Monitoring, vec![])],
        );
        s.put_all_fields(A, vec![field(field_id::btm3(0x30), "ShutDown")]);
        s.put_value(A, f, Value::Float(12.5));

        s.forget_schema(A);

        assert!(s.schema(A).is_none());
        assert!(!s.has_menu(A, Menu::Monitoring));
        assert!(!s.has_all_fields(A));
        assert!(s.has_identity(A));
        assert_eq!(s.identity(A).unwrap().name, "Combi");
        assert_eq!(s.get_value(A, f).unwrap().value, Value::Float(12.5));
    }

    #[test]
    fn forgetting_an_unknown_devices_schema_is_a_no_op() {
        let s = State::new();
        s.forget_schema(A);
        assert!(!s.any_device());
    }

    /// A catalog miss has to be remembered as *attempted*, or every string
    /// fetch would re-run the spot-check.
    #[test]
    fn a_catalog_miss_is_still_recorded_as_attempted() {
        let s = State::new();
        assert!(!s.catalog_attempted(A));
        assert!(s.catalog_table(A).is_none());

        s.put_catalog(A, None);
        assert!(s.catalog_attempted(A));
        assert!(s.catalog_table(A).is_none());
    }

    #[test]
    fn an_access_level_round_trips() {
        let s = State::new();
        assert!(s.access_level(A).is_none());
        s.put_access_level(A, AccessLevel::Installer);
        assert_eq!(s.access_level(A), Some(AccessLevel::Installer));
        s.put_access_level(A, AccessLevel::EndUser);
        assert_eq!(s.access_level(A), Some(AccessLevel::EndUser));
    }

    /// Alive ids are address-sorted (the TUI and the event-target index space
    /// both depend on a stable order) and windowed by the liveness period.
    #[test]
    fn alive_ids_are_sorted_and_windowed() {
        let s = State::new();
        s.mark_alive(B, 0, 0);
        s.mark_alive(A, 0, 0);
        assert_eq!(s.alive_ids(ALIVE), vec![A, B]);

        std::thread::sleep(Duration::from_millis(5));
        assert!(s.alive_ids(Duration::from_millis(1)).is_empty());
        assert_eq!(s.status(A, Duration::from_millis(1)), DeviceStatus::Offline);

        // Still known, just not alive — a later frame brings it back.
        s.touch(A);
        assert_eq!(s.alive_ids(ALIVE), vec![A, B]);
    }

    #[test]
    fn an_unheard_device_is_offline() {
        let s = State::new();
        assert_eq!(s.status(A, ALIVE), DeviceStatus::Offline);
        assert!(s.alive_ids(ALIVE).is_empty());
        assert!(s.newest_first_seen().is_none());
    }

    /// `newest_first_seen` drives the discovery settle window: it must track
    /// the most recent *arrival*, not the most recent frame.
    #[test]
    fn newest_first_seen_tracks_the_latest_arrival() {
        let s = State::new();
        s.mark_alive(A, 0, 0);
        let after_first = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        s.mark_alive(B, 0, 0);

        let newest = s.newest_first_seen().unwrap();
        assert!(newest > after_first);

        // Touching the older device is new traffic, not a new arrival.
        s.touch(A);
        assert_eq!(s.newest_first_seen().unwrap(), newest);
    }
}
