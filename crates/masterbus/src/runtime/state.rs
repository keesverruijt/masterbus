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
}

impl DeviceEntry {
    fn new(addr: u32, now: Instant) -> Self {
        DeviceEntry {
            addr,
            last_seen: now,
            type_code: 0,
            fw_hint: 0,
            identity: None,
            access_level: None,
            schema: None,
            menus: HashSet::new(),
            all_fields: None,
            values: HashMap::new(),
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
        State { devices: Mutex::new(HashMap::new()) }
    }

    /// Mark a device alive (from a broadcast), updating its identity hints.
    pub fn mark_alive(&self, addr: u32, type_code: u8, fw_hint: u16) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
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
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
        e.last_seen = now;
    }

    /// Record a freshly-observed value.
    pub fn put_value(&self, addr: u32, field: FieldId, value: Value) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
        e.values.insert(field, CachedValue { value, at: now, outdated: false });
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
        self.devices.lock().unwrap().get(&addr).and_then(|e| e.values.get(&field).cloned())
    }

    /// Store a device's identity (cheap discovery result).
    pub fn put_identity(&self, addr: u32, identity: DeviceIdentity) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
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
        e.schema.as_ref().map(|s| s.identity()).or_else(|| e.identity.clone())
    }

    /// Get the device's last-known access level. `None` until we've heard
    /// one (either via an explicit read or after a successful login/logout).
    pub fn access_level(&self, addr: u32) -> Option<AccessLevel> {
        self.devices.lock().unwrap().get(&addr).and_then(|e| e.access_level)
    }

    /// Record the device's access level (after a successful read or
    /// login/logout response).
    pub fn put_access_level(&self, addr: u32, level: AccessLevel) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
        e.access_level = Some(level);
    }

    /// Add a discovered menu's groups to a device's schema (identity must already
    /// be stored). Replaces any previously-held groups for that menu.
    pub fn put_menu(&self, addr: u32, menu: Menu, groups: Vec<GroupInfo>) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
        let id = e.identity.clone().unwrap_or_else(|| DeviceIdentity {
            article: String::new(),
            serial: String::new(),
            revision: String::new(),
            name: String::new(),
            firmware: String::new(),
        });
        let schema = e.schema.get_or_insert_with(|| DeviceSchema::from_identity(id, Vec::new()));
        schema.groups.retain(|g| g.menu != menu);
        schema.groups.extend(groups);
        e.menus.insert(menu);
    }

    /// Whether a specific menu has been discovered for a device.
    pub fn has_menu(&self, addr: u32, menu: Menu) -> bool {
        self.devices.lock().unwrap().get(&addr).map(|e| e.menus.contains(&menu)).unwrap_or(false)
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
        self.devices.lock().unwrap().get(&addr).and_then(|e| e.schema.clone())
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
        self.devices.lock().unwrap().get(&addr).is_some_and(|e| e.all_fields.is_some())
    }

    /// Clone the flat field enumeration for a device, if discovered.
    pub fn all_fields(&self, addr: u32) -> Option<Vec<FieldInfo>> {
        self.devices.lock().unwrap().get(&addr).and_then(|e| e.all_fields.clone())
    }

    /// Store the flat field enumeration for a device.
    pub fn put_all_fields(&self, addr: u32, fields: Vec<FieldInfo>) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
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
