//! Lazy, single-threaded device discovery (runs on the scheduler thread).
//!
//! Identity (firmware + strings) is always fetched (cheap). The expensive group/
//! field/shadow enumeration is keyed by `article + firmware` and may be loaded
//! from / persisted to the optional on-disk cache.

use std::path::Path;

use super::waiter::Waiter;
use super::Config;
use crate::model::{DeviceIdentity, DeviceSchema, FieldInfo, GroupInfo, Menu};
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

    fn shadow_u16(&mut self, addr: u32, op: u8, field: u8, retries: usize) -> Option<u16> {
        let key = format!("shadow:{:06X}:{:02X}:{}", addr, op, field);
        self.req(&key, shadow_meta_req_raw(addr, op, field as u16), retries)
            .filter(|r| r.len() >= 6)
            .map(|r| u16::from_le_bytes([r[4], r[5]]))
    }

    fn shadow_byte4(&mut self, addr: u32, op: u8, field: u8) -> Option<u8> {
        let key = format!("shadow:{:06X}:{:02X}:{}", addr, op, field);
        self.req_std(&key, shadow_meta_req_raw(addr, op, field as u16))
            .and_then(|r| r.get(4).copied())
    }

    fn shadow_f32(&mut self, addr: u32, op: u8, field: u8) -> Option<f32> {
        let key = format!("shadow:{:06X}:{:02X}:{}", addr, op, field);
        self.req_std(&key, shadow_meta_req_raw(addr, op, field as u16))
            .filter(|r| r.len() >= 8)
            .map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]))
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

/// Enumerate (or load from the `article+firmware` cache) a device's groups. The
/// expensive half of discovery.
pub(super) fn discover_groups(disc: &mut Disc, addr: u32, id: &DeviceIdentity) -> Vec<GroupInfo> {
    load_cached_groups(disc.cfg.cache_path.as_deref(), &id.article, &id.firmware).unwrap_or_else(
        || {
            let g = enumerate_groups(disc, addr);
            store_cached_groups(disc.cfg.cache_path.as_deref(), &id.article, &id.firmware, &g);
            g
        },
    )
}

/// Discover a device's full schema (identity + groups).
pub(super) fn discover(disc: &mut Disc, addr: u32) -> DeviceSchema {
    let id = fetch_identity(disc, addr);
    let groups = discover_groups(disc, addr, &id);
    DeviceSchema::from_identity(id, groups)
}

fn enumerate_groups(disc: &mut Disc, addr: u32) -> Vec<GroupInfo> {
    let mut groups = Vec::new();
    let mut gid: u32 = 0; // global group id, contiguous across menus
    for (menu, selector) in [
        (Menu::Monitoring, 0x02u8),
        (Menu::Configuration, 0x03u8),
        (Menu::Service, 0x04u8),
    ] {
        let count = disc.group_count(addr, selector);
        for _ in 0..count {
            if let Some(g) = enumerate_group(disc, addr, gid as u8, menu) {
                groups.push(g);
            }
            gid += 1;
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
        if let Some(r) = disc.req_std(&key, schema_field_id_req_raw(addr, g, idx as u8)) {
            if r.len() >= 6 {
                field_ids.push(u16::from_le_bytes([r[4], r[5]]) as i32);
            }
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
    let f = fid as u8;
    let name_sid = disc.shadow_u16(addr, 0x28, f, disc.cfg.discovery_retries);
    let viz_code = disc.shadow_byte4(addr, 0x02, f).unwrap_or(0x01);
    let n_or_max = disc.shadow_f32(addr, 0x07, f).unwrap_or(0.0);
    let unit_sid = disc.shadow_u16(addr, 0x2C, f, OPTIONAL_RETRIES);
    let min = disc.shadow_f32(addr, 0x06, f).unwrap_or(0.0);
    let step = disc.shadow_f32(addr, 0x08, f).unwrap_or(0.0);
    // Writability: shadow op 0x0B, flag at byte[4].
    let writeable = disc.shadow_byte4(addr, 0x0B, f).map(|b| b != 0).unwrap_or(false);

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
        min: min as f64,
        max: n_or_max as f64,
        step: step as f64,
        options,
    }
}

// ── disk cache (groups keyed by article+firmware) ───────────────────────────

fn cache_file(dir: &Path, article: &str, firmware: &str) -> std::path::PathBuf {
    let safe: String = DeviceSchema::cache_key(article, firmware)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    dir.join(format!("{}.json", safe))
}

fn load_cached_groups(dir: Option<&Path>, article: &str, firmware: &str) -> Option<Vec<GroupInfo>> {
    let dir = dir?;
    if article.is_empty() {
        return None;
    }
    let data = std::fs::read(cache_file(dir, article, firmware)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn store_cached_groups(dir: Option<&Path>, article: &str, firmware: &str, groups: &[GroupInfo]) {
    let Some(dir) = dir else { return };
    if article.is_empty() || groups.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec_pretty(groups) {
        let _ = std::fs::write(cache_file(dir, article, firmware), json);
    }
}
