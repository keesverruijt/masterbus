//! Lazy, single-threaded device discovery (runs on the scheduler thread).
//!
//! Identity (firmware + strings) is always fetched (cheap). The expensive group/
//! field/shadow enumeration is cached **per device** (keyed by serial) and may be
//! loaded from / persisted to the optional on-disk cache.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::waiter::Waiter;
use super::Config;
use crate::model::{DeviceIdentity, FieldInfo, GroupInfo, Menu};
use crate::protocol::{
    fw_req_raw, group_count_req_raw, prop_str_id_req_raw, schema_field_count_req_raw,
    schema_field_id_req_raw, schema_group_name_req_raw, shadow_meta_req_raw, shadow_option_req_raw,
    string_chunk_req_raw, viz_from_wire, VisualizationType,
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

    /// Fetch several shadow metadata ops for one field in a single round-trip:
    /// register all keys, fire all requests back-to-back, then collect within one
    /// shared timeout window. Absent metadata (silent or a `0x10` "no value")
    /// resolves within that one window instead of timing out per op — this is the
    /// main discovery speed-up. Returns op → raw response payload.
    fn shadow_batch(&mut self, addr: u32, field: u8, ops: &[u8]) -> HashMap<u8, Vec<u8>> {
        let key = |op: u8| format!("shadow:{:06X}:{:02X}:{}", addr, op, field);
        for &op in ops {
            self.waiter.register(&key(op));
        }
        for &op in ops {
            let frame = shadow_meta_req_raw(addr, op, field as u16);
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
/// half of discovery — no group/field/shadow enumeration.
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

/// The menus enumerated for a "full" discovery, in global group-id order.
pub(super) const MENUS: [Menu; 3] = [Menu::Monitoring, Menu::Configuration, Menu::Service];

/// Base global group id of `menu` (number of groups in the menus before it) and
/// the count of groups in `menu` itself. Groups are a single global list ordered
/// monitoring → config → service.
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
    for i in 0..count {
        let gid = (offset + i) as u8;
        if let Some(g) = enumerate_group(disc, addr, gid, menu) {
            groups.push(g);
        }
    }
    groups
}

fn enumerate_group(disc: &mut Disc, addr: u32, g: u8, menu: Menu) -> Option<GroupInfo> {
    let name_sid = {
        let key = format!("schema:{:06X}:28:{}", addr, g);
        disc.req_std(&key, schema_group_name_req_raw(addr, g))
            .filter(|r| r.len() >= 6)
            .map(|r| u16::from_le_bytes([r[4], r[5]]))
    };
    let field_count = {
        let key = format!("schema:{:06X}:07:{}", addr, g);
        disc.req_std(&key, schema_field_count_req_raw(addr, g))
            .filter(|r| r.len() >= 8)
            .map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]) as u32)
            .unwrap_or(0)
    };

    let mut field_ids: Vec<i32> = Vec::new();
    for idx in 0..field_count {
        let key = format!("schema:{:06X}:03:{}:{}", addr, g, idx as u8);
        if let Some(r) = disc.req_std(&key, schema_field_id_req_raw(addr, g, idx as u8))
            && r.len() >= 6
        {
            field_ids.push(u16::from_le_bytes([r[4], r[5]]) as i32);
        }
    }

    let name = name_sid.map(|sid| disc.fetch_str(addr, sid)).unwrap_or_default();
    let mut fields = Vec::new();
    for fid in field_ids {
        fields.push(enumerate_field(disc, addr, fid));
    }
    Some(GroupInfo { id: g as i32, name, menu, fields })
}

fn enumerate_field(disc: &mut Disc, addr: u32, fid: i32) -> FieldInfo {
    use crate::protocol::shadow_op as op;
    let f = fid as u8;

    // Per-field metadata in one pipelined round-trip. Numeric editing bounds
    // (MIN/STEP) are deferred — they aren't needed to enumerate or read a field,
    // so we skip them here (op::MAX is still fetched: it doubles as the option
    // count for lists).
    let meta = disc.shadow_batch(addr, f, &[op::NAME, op::VIZ, op::MAX, op::UNIT, op::WRITEABLE]);
    let u16_at4 = |o: u8| meta.get(&o).filter(|r| r.len() >= 6).map(|r| u16::from_le_bytes([r[4], r[5]]));
    let byte4 = |o: u8| meta.get(&o).and_then(|r| r.get(4).copied());
    let f32_at4 = |o: u8| {
        meta.get(&o).filter(|r| r.len() >= 8).map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]))
    };

    let name_sid = u16_at4(op::NAME);
    let viz_code = byte4(op::VIZ).unwrap_or(0x01);
    let n_or_max = f32_at4(op::MAX).unwrap_or(0.0);
    let unit_sid = u16_at4(op::UNIT);
    // Writability: shadow op 0x0B, flag at byte[4].
    let writeable = byte4(op::WRITEABLE).map(|b| b != 0).unwrap_or(false);

    let viz = viz_from_wire(viz_code);
    let n_opts = n_or_max as u32;

    let options = if matches!(viz, VisualizationType::DropDown | VisualizationType::Eventable)
        && n_opts > 0
        && n_opts <= 64
    {
        let mut opts = Vec::new();
        for opt in 0..n_opts {
            let key = format!("shadow:{:06X}:26:{}:{}", addr, f, opt as u8);
            let osid = disc
                .req(&key, shadow_option_req_raw(addr, f, opt as u8), OPTIONAL_RETRIES)
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

    FieldInfo {
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
    }
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
