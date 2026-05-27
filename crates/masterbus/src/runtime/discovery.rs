//! Lazy, single-threaded device discovery (runs on the scheduler thread).
//!
//! Identity (firmware + strings) is always fetched (cheap). The expensive group/
//! field/metadata enumeration is cached **per device** (keyed by serial) and may be
//! loaded from / persisted to the optional on-disk cache.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::waiter::Waiter;
use super::Config;
use crate::model::{DeviceIdentity, FieldInfo, GroupInfo, Menu};
use crate::protocol::{
    btm1_meta_option_req_raw, btm1_meta_req_raw, can_class, fw_req_raw, group_count_req_raw,
    prop_str_id_req_raw, schema_field_count_req_class_raw, schema_field_id_req_class_raw,
    schema_group_name_req_class_raw, string_chunk_req_raw, viz_from_wire, VisualizationType,
};
use crate::transport::TransportTx;

const OPTIONAL_RETRIES: usize = 1;

/// Bundle of what discovery needs from the scheduler.
pub(super) struct Disc<'a> {
    pub tx: &'a mut dyn TransportTx,
    pub waiter: &'a Waiter,
    pub cfg: &'a Config,
}

impl Disc<'_> {
    fn req(&mut self, key: &str, frame: (u32, Vec<u8>), retries: usize) -> Option<Vec<u8>> {
        for _ in 0..retries {
            self.waiter.register(key);
            let _ = self.tx.send(frame.0, &frame.1);
            if let Some(r) = self.waiter.wait(key, self.cfg.discovery_timeout) {
                return Some(r);
            }
        }
        None
    }

    fn req_std(&mut self, key: &str, frame: (u32, Vec<u8>)) -> Option<Vec<u8>> {
        let n = self.cfg.discovery_retries;
        self.req(key, frame, n)
    }

    /// Fetch a string by id via chunked reads.
    fn fetch_str(&mut self, addr: u32, str_id: u16) -> String {
        let mut s = String::new();
        let mut seq: u8 = 0;
        loop {
            let key = format!("str:{:06X}:{:04X}:{}", addr, str_id, seq);
            let r = match self.req_std(&key, string_chunk_req_raw(addr, str_id, seq)) {
                Some(r) => r,
                None => break,
            };
            let last = r.len() < 8;
            for &c in &r[4..r.len()] {
                if c == 0 {
                    return s;
                }
                s.push(c as char);
            }
            if last {
                break;
            }
            seq = seq.wrapping_add(1);
        }
        s
    }

    /// Property string id (`None` if no response; `Some(0)` is a valid id).
    fn prop_str_id(&mut self, addr: u32, n: u8) -> Option<u16> {
        let key = format!("p:{:06X}:09:{:02X}", addr, n);
        self.req_std(&key, prop_str_id_req_raw(addr, n))
            .filter(|r| r.len() >= 4)
            .map(|r| u16::from_le_bytes([r[2], r[3]]))
    }

    /// Fetch several Btm1 per-field metadata ops in a single round-trip:
    /// register all keys, fire all requests back-to-back, then collect within
    /// one shared timeout window. Absent metadata (silent or a `0x10` "no
    /// value") resolves within that one window instead of timing out per op —
    /// this is the main discovery speed-up. Returns op → raw response payload.
    ///
    /// **Only the Btm1 metadata channel** (`0x18` → `0x08` to `addr | 0x800000`)
    /// is queried. The Btm3 channel (class `0x1C`) exposes a **separate field
    /// namespace** with different names/values per index; the in-flight
    /// redesign moves Btm3 onto its own field-id space so the two can coexist.
    fn btm1_meta_batch(&mut self, addr: u32, field: u8, ops: &[u8]) -> HashMap<u8, Vec<u8>> {
        let key = |op: u8| format!("btm1_meta:{:06X}:{:02X}:{}", addr, op, field);
        for &op in ops {
            self.waiter.register(&key(op));
        }
        for &op in ops {
            let frame = btm1_meta_req_raw(addr, op, field as u16);
            let _ = self.tx.send(frame.0, &frame.1);
        }
        let deadline = Instant::now() + self.cfg.discovery_timeout;
        let mut out = HashMap::with_capacity(ops.len());
        for &op in ops {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(r) = self.waiter.wait(&key(op), remaining.max(Duration::from_millis(1))) {
                out.insert(op, r);
            }
        }
        out
    }

    fn group_count(&mut self, addr: u32, selector: u8) -> u32 {
        let key = format!("p:{:06X}:08:{:02X}", addr, selector);
        self.req_std(&key, group_count_req_raw(addr, selector))
            .filter(|r| r.len() >= 4)
            .map(|r| u16::from_le_bytes([r[2], r[3]]) as u32)
            .unwrap_or(0)
    }
}

/// Fetch a device's identity (firmware + property strings). This is the cheap
/// half of discovery — no group/field/metadata enumeration.
pub(super) fn fetch_identity(disc: &mut Disc, addr: u32) -> DeviceIdentity {
    let firmware = {
        let k0 = format!("p:{:06X}:82:00", addr);
        let k1 = format!("p:{:06X}:82:01", addr);
        let r0 = disc.req_std(&k0, fw_req_raw(addr, 0)).unwrap_or_default();
        let r1 = disc.req_std(&k1, fw_req_raw(addr, 1)).unwrap_or_default();
        let major = r0.get(3).copied().unwrap_or(0) as u32 + r1.get(3).copied().unwrap_or(0) as u32;
        let minor = r0.get(2).copied().unwrap_or(0) as u32 + r1.get(2).copied().unwrap_or(0) as u32;
        format!("{}.{}", major, minor)
    };
    let fetch_prop = |disc: &mut Disc, n: u8| -> String {
        disc.prop_str_id(addr, n).map(|sid| disc.fetch_str(addr, sid)).unwrap_or_default()
    };
    let article = fetch_prop(disc, 1);
    let serial = fetch_prop(disc, 2);
    let name = fetch_prop(disc, 3);
    let revision = {
        let r = fetch_prop(disc, 4);
        if r.is_empty() {
            serial.chars().nth(4).map(String::from).unwrap_or_default()
        } else {
            r
        }
    };
    DeviceIdentity { article, serial, revision, name, firmware }
}

/// The menus enumerated for a "full" discovery. Monitoring / Configuration /
/// Service share one global gid space (0..mon+cfg+svc). Alarm and History use
/// their own gid namespaces (each starting at 0) on parallel schema channels.
pub(super) const MENUS: [Menu; 5] = [
    Menu::Monitoring,
    Menu::Configuration,
    Menu::Service,
    Menu::Alarm,
    Menu::History,
];

/// Maximum gid we'll probe-and-stop on for menus without a known count
/// selector (Alarm without confirmed `sel_08`, History always).
const PROBE_GID_MAX: u8 = 16;

/// Waiter-key prefix for the schema response that pairs with a given request
/// class — must agree with the routing in `protocol::decode::waiter_key_for_frame`.
fn schema_waiter_prefix(class: u8) -> &'static str {
    match class {
        can_class::SCHEMA_REQ_ALARM => "schema_alarm",
        can_class::SCHEMA_REQ_HISTORY => "schema_history",
        _ => "schema", // SCHEMA_REQ and any future variants land in the default key family
    }
}

/// Base global group id of `menu` and the count of groups in `menu` itself.
/// For Monitoring/Configuration/Service the gid is global (offset by the prior
/// menus); for Alarm/History the gid starts at 0 in the menu's own namespace
/// and the offset returned here is 0.
fn menu_range(disc: &mut Disc, addr: u32, menu: Menu) -> (u32, u32) {
    match menu {
        Menu::Monitoring => (0, disc.group_count(addr, 0x02)),
        Menu::Configuration => {
            let mon = disc.group_count(addr, 0x02);
            (mon, disc.group_count(addr, 0x03))
        }
        Menu::Service => {
            let mon = disc.group_count(addr, 0x02);
            let cfg = disc.group_count(addr, 0x03);
            (mon + cfg, disc.group_count(addr, 0x04))
        }
        // Alarm/History gid namespaces are independent and don't always
        // expose their count via `[0x08, sel]`. `selector()` returns the
        // hypothesised selector (0x08 for Alarm); when zero or unreliable
        // we fall back to probe-and-stop inside `enumerate_menu`.
        Menu::Alarm | Menu::History => (0, 0),
        Menu::Other(_) => (0, 0),
    }
}

/// Discover just one menu's groups: per-device disk cache, else enumerate live.
///
/// The cache is keyed by **serial** (per device), not article+firmware: devices
/// that share an article/firmware can still have different schemas — e.g. one
/// battery in a cluster exposes an extra "Cluster" group the others don't — so
/// sharing a schema across them would hide those differences.
pub(super) fn discover_menu(
    disc: &mut Disc,
    addr: u32,
    id: &DeviceIdentity,
    menu: Menu,
) -> Vec<GroupInfo> {
    load_cached_menu(disc.cfg.cache_path.as_deref(), &id.serial, &id.firmware, menu).unwrap_or_else(
        || {
            let g = enumerate_menu(disc, addr, menu);
            store_cached_menu(disc.cfg.cache_path.as_deref(), &id.serial, &id.firmware, menu, &g);
            g
        },
    )
}

fn enumerate_menu(disc: &mut Disc, addr: u32, menu: Menu) -> Vec<GroupInfo> {
    let (offset, count) = menu_range(disc, addr, menu);
    let mut groups = Vec::new();
    if count > 0 {
        for i in 0..count {
            let gid = (offset + i) as u8;
            if let Some(g) = enumerate_group(disc, addr, gid, menu) {
                groups.push(g);
            }
        }
    } else if matches!(menu, Menu::Alarm | Menu::History) {
        // Probe-and-stop on the menu's own schema channel — stop after two
        // consecutive misses or when we hit PROBE_GID_MAX.
        let mut misses = 0;
        for gid in 0..PROBE_GID_MAX {
            match enumerate_group(disc, addr, gid, menu) {
                Some(g) => {
                    misses = 0;
                    groups.push(g);
                }
                None => {
                    misses += 1;
                    if misses >= 2 {
                        break;
                    }
                }
            }
        }
    }
    groups
}

fn enumerate_group(disc: &mut Disc, addr: u32, g: u8, menu: Menu) -> Option<GroupInfo> {
    let class = menu.schema_request_class();
    let key_prefix = schema_waiter_prefix(class);
    let name_sid = {
        let key = format!("{}:{:06X}:28:{}", key_prefix, addr, g);
        disc.req_std(&key, schema_group_name_req_class_raw(class, addr, g))
            .filter(|r| r.len() >= 6)
            .map(|r| u16::from_le_bytes([r[4], r[5]]))
    };
    let field_count = {
        let key = format!("{}:{:06X}:07:{}", key_prefix, addr, g);
        disc.req_std(&key, schema_field_count_req_class_raw(class, addr, g))
            .filter(|r| r.len() >= 8)
            .map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]) as u32)
            .unwrap_or(0)
    };
    // For Alarm/History, "no name and no fields" likely means the gid is
    // unallocated — the probe-and-stop loop above relies on this to terminate.
    if matches!(menu, Menu::Alarm | Menu::History) && name_sid.is_none() && field_count == 0 {
        return None;
    }

    let mut field_ids: Vec<i32> = Vec::new();
    for idx in 0..field_count {
        let key = format!("{}:{:06X}:03:{}:{}", key_prefix, addr, g, idx as u8);
        if let Some(r) = disc.req_std(&key, schema_field_id_req_class_raw(class, addr, g, idx as u8))
            && r.len() >= 6
        {
            field_ids.push(u16::from_le_bytes([r[4], r[5]]) as i32);
        }
    }

    let name = name_sid.map(|sid| disc.fetch_str(addr, sid)).unwrap_or_default();
    let mut fields = Vec::new();
    for fid in field_ids {
        if let Some(f) = enumerate_field(disc, addr, fid) {
            fields.push(f);
        }
    }
    Some(GroupInfo { id: g as i32, name, menu, fields })
}

/// Probe every field index in the device's full index space (selector `0x01`,
/// see PROTOCOL.md §4.3 + the Nav Chg / TDevice_MacMagic discussion in
/// FINDINGS), dropping indices that don't respond to any metadata query.
///
/// This is the only way to enumerate Configuration items on devices whose
/// `0x08 0x03` group count lies (e.g. the Magic-class Nav Chg, which reports
/// zero config groups despite having ~25 settings). The grouping that
/// MasterAdjust shows comes from a hard-coded per-device-family layout
/// (`TDevice_MacMagic` etc. inside `MasterAdjust.exe`); the wire protocol
/// itself only exposes a flat field-index space.
pub(super) fn enumerate_all_fields(disc: &mut Disc, addr: u32) -> Vec<FieldInfo> {
    let n = disc.group_count(addr, 0x01) as i32;
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n as usize);
    for fid in 0..n {
        if let Some(f) = enumerate_field(disc, addr, fid) {
            out.push(f);
        }
    }
    out
}

fn enumerate_field(disc: &mut Disc, addr: u32, fid: i32) -> Option<FieldInfo> {
    use crate::protocol::meta_op as op;
    let f = fid as u8;

    // Per-field metadata in one pipelined round-trip. Numeric editing bounds
    // (MIN/STEP) are deferred — they aren't needed to enumerate or read a field,
    // so we skip them here (op::MAX is still fetched: it doubles as the option
    // count for lists).
    let meta = disc.btm1_meta_batch(addr, f, &[op::NAME, op::VIZ, op::MAX, op::UNIT, op::WRITEABLE]);
    // If nothing came back, this field index is unallocated — used by the
    // `enumerate_all_fields` flat probe to skip holes in the index space.
    if meta.is_empty() {
        return None;
    }
    let u16_at4 = |o: u8| meta.get(&o).filter(|r| r.len() >= 6).map(|r| u16::from_le_bytes([r[4], r[5]]));
    let byte4 = |o: u8| meta.get(&o).and_then(|r| r.get(4).copied());
    let f32_at4 = |o: u8| {
        meta.get(&o).filter(|r| r.len() >= 8).map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]))
    };

    let name_sid = u16_at4(op::NAME);
    let viz_code = byte4(op::VIZ).unwrap_or(0x01);
    let n_or_max = f32_at4(op::MAX).unwrap_or(0.0);
    let unit_sid = u16_at4(op::UNIT);
    // Writability: meta op 0x0B, flag at byte[4].
    let writeable = byte4(op::WRITEABLE).map(|b| b != 0).unwrap_or(false);

    let viz = viz_from_wire(viz_code);
    let n_opts = n_or_max as u32;

    let options = if matches!(viz, VisualizationType::DropDown | VisualizationType::Eventable)
        && n_opts > 0
        && n_opts <= 64
    {
        let mut opts = Vec::new();
        for opt in 0..n_opts {
            let key = format!("btm1_meta:{:06X}:26:{}:{}", addr, f, opt as u8);
            // Btm1 channel only — Btm3 option strings will be added in a
            // separate, channel-aware code path.
            let osid = disc
                .req(&key, btm1_meta_option_req_raw(addr, f, opt as u8), OPTIONAL_RETRIES)
                .filter(|r| r.len() >= 6)
                .map(|r| u16::from_le_bytes([r[4], r[5]]));
            opts.push(osid.filter(|&s| s != 0).map(|s| disc.fetch_str(addr, s)).unwrap_or_default());
        }
        opts
    } else {
        Vec::new()
    };

    let field_name = name_sid.filter(|&s| s != 0).map(|s| disc.fetch_str(addr, s)).unwrap_or_default();
    let field_unit = unit_sid.filter(|&s| s != 0).map(|s| disc.fetch_str(addr, s)).unwrap_or_default();

    Some(FieldInfo {
        index: fid,
        name: field_name,
        unit: field_unit,
        viz_type: viz,
        writeable,
        // Deferred numeric bounds (not fetched during enumeration).
        min: 0.0,
        max: n_or_max as f64,
        step: 0.0,
        options,
    })
}

// ── disk cache (groups keyed by serial+firmware+menu, i.e. per device) ────────

fn cache_file(dir: &Path, serial: &str, firmware: &str, menu: Menu) -> std::path::PathBuf {
    let key = format!("{}-{}-{:02x}", serial, firmware, menu.selector());
    let safe: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    dir.join(format!("{}.json", safe))
}

fn load_cached_menu(
    dir: Option<&Path>,
    serial: &str,
    firmware: &str,
    menu: Menu,
) -> Option<Vec<GroupInfo>> {
    let dir = dir?;
    if serial.is_empty() {
        return None;
    }
    let data = std::fs::read(cache_file(dir, serial, firmware, menu)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn store_cached_menu(dir: Option<&Path>, serial: &str, firmware: &str, menu: Menu, groups: &[GroupInfo]) {
    let Some(dir) = dir else { return };
    if serial.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec_pretty(groups) {
        let _ = std::fs::write(cache_file(dir, serial, firmware, menu), json);
    }
}
