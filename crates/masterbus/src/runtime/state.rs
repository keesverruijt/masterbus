//! Shared in-memory state: device liveness, discovered schema, value cache.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use crate::model::{DeviceIdentity, DeviceSchema, DeviceStatus, GroupInfo, Menu};
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
    /// Schema discovered so far; `groups` accumulates menu-by-menu as menus are
    /// lazily discovered. `None` until at least the identity is known.
    pub schema: Option<DeviceSchema>,
    /// Which menus have been discovered into `schema.groups`.
    pub menus: HashSet<Menu>,
    /// Latest value per field index.
    pub values: HashMap<i32, CachedValue>,
}

impl DeviceEntry {
    fn new(addr: u32, now: Instant) -> Self {
        DeviceEntry {
            addr,
            last_seen: now,
            type_code: 0,
            fw_hint: 0,
            identity: None,
            schema: None,
            menus: HashSet::new(),
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

    /// Touch last-seen for any frame from a device (without identity hints).
    pub fn touch(&self, addr: u32) {
        let now = Instant::now();
        if let Some(e) = self.devices.lock().unwrap().get_mut(&addr) {
            e.last_seen = now;
        }
    }

    /// Record a freshly-observed value.
    pub fn put_value(&self, addr: u32, field: i32, value: Value) {
        let now = Instant::now();
        let mut map = self.devices.lock().unwrap();
        let e = map.entry(addr).or_insert_with(|| DeviceEntry::new(addr, now));
        e.values.insert(field, CachedValue { value, at: now, outdated: false });
    }

    /// Mark a field's cached value outdated (e.g. after a write).
    pub fn mark_outdated(&self, addr: u32, field: i32) {
        if let Some(e) = self.devices.lock().unwrap().get_mut(&addr) {
            if let Some(v) = e.values.get_mut(&field) {
                v.outdated = true;
            }
        }
    }

    /// Get a cached value (clone) if present.
    pub fn get_value(&self, addr: u32, field: i32) -> Option<CachedValue> {
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
    pub fn has_field(&self, addr: u32, field: i32) -> bool {
        let map = self.devices.lock().unwrap();
        map.get(&addr)
            .and_then(|e| e.schema.as_ref())
            .map(|s| s.field(field).is_some())
            .unwrap_or(false)
    }

    /// Clone a device's schema-so-far (groups of all discovered menus).
    pub fn schema(&self, addr: u32) -> Option<DeviceSchema> {
        self.devices.lock().unwrap().get(&addr).and_then(|e| e.schema.clone())
    }

    /// Find one menu's groups from an already-discovered device with the same
    /// identity key (`article + firmware`). Lets identical devices discover each
    /// menu once and reuse it.
    pub fn menu_groups_by_key(
        &self,
        article: &str,
        firmware: &str,
        menu: Menu,
    ) -> Option<Vec<GroupInfo>> {
        if article.is_empty() {
            return None;
        }
        let map = self.devices.lock().unwrap();
        map.values().find_map(|e| {
            let s = e.schema.as_ref()?;
            if s.article == article && s.firmware == firmware && e.menus.contains(&menu) {
                Some(s.groups.iter().filter(|g| g.menu == menu).cloned().collect())
            } else {
                None
            }
        })
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
