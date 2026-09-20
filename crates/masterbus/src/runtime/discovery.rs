//! Lazy, single-threaded device discovery (runs on the scheduler thread).
//!
//! Identity (firmware + strings) is always fetched (cheap). The expensive group/
//! field/metadata enumeration is cached **per device** (keyed by serial) and may be
//! loaded from / persisted to the optional on-disk cache.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::Config;
use super::framelog::frame_log;
use super::waiter::Waiter;
use crate::model::{
    AccessLevel, Channel, DeviceId, DeviceIdentity, FieldId, FieldInfo, GroupInfo, Menu, field_id,
};
use crate::protocol::{
    VisualizationType, btm1_meta_option_req_raw, btm1_meta_req_raw, btm3_meta_option_req_raw,
    btm3_meta_req_raw, can_class, fw_req_raw, group_count_req_raw, prop_str_id_req_raw,
    schema_field_count_req_class_raw, schema_field_id_req_class_raw,
    schema_group_name_req_class_raw, string_chunk_req_raw, viz_from_wire,
};
use crate::transport::TransportTx;

const OPTIONAL_RETRIES: usize = 1;

/// How long a best-effort metadata op ([`Disc::meta_batch`]) is polled after
/// the blocking ops resolve. Its response, if any, has already arrived with the
/// batch, so this only needs to cover jitter — not a full discovery timeout.
const BEST_EFFORT_META_WAIT: Duration = Duration::from_millis(4);

/// Wire-frame encoder for a metadata request: `(addr, opcode, field_id) → frame`.
type MetaEncode = fn(DeviceId, u8, u16) -> (u32, Vec<u8>);
/// Wire-frame encoder for an option-string request: `(addr, field, opt) → frame`.
type MetaOptionEncode = fn(DeviceId, u8, u8) -> (u32, Vec<u8>);

/// Bundle of what discovery needs from the scheduler.
pub(super) struct Disc<'a> {
    pub tx: &'a mut dyn TransportTx,
    pub waiter: &'a Waiter,
    pub cfg: &'a Config,
    /// Access level the disk cache files will be keyed by. Writability
    /// (and a few other attributes) flip per level, so each level's
    /// discovered schema gets its own cache file. Unknown defaults to
    /// `EndUser` (the device's boot state).
    pub level: AccessLevel,
    /// Offline string table for this device, once resolved and spot-checked
    /// (see [`resolve_catalog`]). When set, [`Disc::fetch_str`] serves listed
    /// ids from it instead of the four-chars-per-round-trip wire fetch; ids not
    /// in the table (EEPROM / runtime-generated / out of span) still go live.
    /// `None` until resolved, or when the device has no usable bundled table.
    pub catalog: Option<&'static HashMap<u16, String>>,
}

impl Disc<'_> {
    fn req(&mut self, key: &str, frame: (u32, Vec<u8>), retries: usize) -> Option<Vec<u8>> {
        for attempt in 0..retries {
            self.waiter.register(key);
            frame_log("Tx", frame.0, &frame.1);
            let _ = self.tx.send(frame.0, &frame.1);
            if let Some(r) = self.waiter.wait(key, self.cfg.discovery_timeout) {
                if attempt > 0 {
                    log::debug!(target: "masterbus::discovery", "{key}: ok on retry {attempt}");
                }
                return Some(r);
            }
            log::debug!(
                target: "masterbus::discovery",
                "{key}: timeout after {:?} (attempt {}/{retries})",
                self.cfg.discovery_timeout, attempt + 1
            );
        }
        log::debug!(
            target: "masterbus::discovery",
            "{key}: giving up after {retries} attempt(s)"
        );
        None
    }

    fn req_std(&mut self, key: &str, frame: (u32, Vec<u8>)) -> Option<Vec<u8>> {
        let n = self.cfg.discovery_retries;
        self.req(key, frame, n)
    }

    /// Fetch a string by id. Served from the offline catalog when the id is in
    /// this device's spot-checked table (zero round trips); otherwise fetched
    /// live via [`Disc::fetch_str_wire`].
    pub(super) fn fetch_str(&mut self, addr: DeviceId, str_id: u16) -> String {
        if let Some(cat) = self.catalog {
            if let Some(s) = cat.get(&str_id) {
                return s.clone();
            }
        }
        self.fetch_str_wire(addr, str_id)
    }

    /// Fetch a string by id via chunked wire reads (opcode `0x30`, four chars
    /// per round trip). Bypasses the catalog — used for the resolution
    /// spot-check and for any id the catalog doesn't cover.
    pub(super) fn fetch_str_wire(&mut self, addr: DeviceId, str_id: u16) -> String {
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
    fn prop_str_id(&mut self, addr: DeviceId, n: u8) -> Option<u16> {
        let key = format!("p:{:06X}:09:{:02X}", addr, n);
        self.req_std(&key, prop_str_id_req_raw(addr, n))
            .filter(|r| r.len() >= 4)
            .map(|r| u16::from_le_bytes([r[2], r[3]]))
    }

    /// Fetch several per-field metadata ops in one pipelined round-trip on
    /// the given channel. Returns op → raw response payload.
    ///
    /// The two channels speak the same opcode set (see [`crate::protocol::meta_op`])
    /// but expose **independent field namespaces** — a Btm1 field id `0x17` is
    /// a different field than a Btm3 field id `0x17`. The caller therefore
    /// passes a channel-tagged [`FieldId`]; this function picks the right
    /// encoder + waiter-key family internally.
    ///
    /// Absent metadata (silent or a `0x10` "no value") resolves within one
    /// shared timeout window — that batching is the main discovery speed-up.
    ///
    /// `best_effort` ops are sent alongside `ops` but **not** waited on with the
    /// full timeout — only briefly polled after the blocking ops resolve. Use
    /// it for an opcode a device answers only for *some* fields (e.g. the
    /// eventable flag `0x0D`, which a device replies to only for its eventable
    /// outputs): a blocking wait would pay a full timeout on every silent field,
    /// but the response — when it comes — arrives together with the other ops,
    /// so it's already present by the time the blocking ops finish.
    fn meta_batch(
        &mut self,
        addr: DeviceId,
        fid: FieldId,
        ops: &[u8],
        best_effort: &[u8],
    ) -> HashMap<u8, Vec<u8>> {
        let wire = field_id::wire_index(fid);
        let (prefix, encode): (&str, MetaEncode) = match field_id::channel(fid) {
            Channel::Btm1 => ("btm1_meta", btm1_meta_req_raw),
            Channel::Btm3 => ("btm3_meta", btm3_meta_req_raw),
        };
        let key = |op: u8| format!("{}:{:06X}:{:02X}:{}", prefix, addr, op, wire);
        for &op in ops.iter().chain(best_effort) {
            self.waiter.register(&key(op));
        }
        for &op in ops.iter().chain(best_effort) {
            let frame = encode(addr, op, wire as u16);
            frame_log("Tx", frame.0, &frame.1);
            let _ = self.tx.send(frame.0, &frame.1);
        }
        let deadline = Instant::now() + self.cfg.discovery_timeout;
        let mut out = HashMap::with_capacity(ops.len() + best_effort.len());
        for &op in ops {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(r) = self
                .waiter
                .wait(&key(op), remaining.max(Duration::from_millis(1)))
            {
                out.insert(op, r);
            }
        }
        // Best-effort ops: a short poll (the response is already in flight with
        // the batch), never the full timeout.
        for &op in best_effort {
            if let Some(r) = self.waiter.wait(&key(op), BEST_EFFORT_META_WAIT) {
                out.insert(op, r);
            }
        }
        out
    }

    /// Pipelined existence probe: which wire indices on this channel have any
    /// field at all? We send a single VIZ query per index in chunks of
    /// [`PROBE_CHUNK`], wait one shared timeout window per chunk, and collect
    /// the indices that responded. Used as the cheap first phase of
    /// [`enumerate_all_fields`] — replaces the previous serial-with-miss-streak
    /// loop, which was paying ~1 timeout per missing index and was tuned too
    /// tightly to bridge the EasyView's `0x42`-index gap between header fields
    /// and the Switch / Message blocks.
    fn probe_existence(&mut self, addr: DeviceId, channel: Channel, max: u16) -> Vec<u8> {
        use crate::protocol::meta_op as op;
        let (prefix, encode): (&str, MetaEncode) = match channel {
            Channel::Btm1 => ("btm1_meta", btm1_meta_req_raw),
            Channel::Btm3 => ("btm3_meta", btm3_meta_req_raw),
        };
        let key = |wire: u16| format!("{}:{:06X}:{:02X}:{}", prefix, addr, op::VIZ, wire);

        let mut existing = Vec::new();
        let mut wire: u16 = 0;
        while wire < max {
            let end = (wire + PROBE_CHUNK).min(max);
            for w in wire..end {
                self.waiter.register(&key(w));
            }
            for w in wire..end {
                let frame = encode(addr, op::VIZ, w);
                frame_log("Tx", frame.0, &frame.1);
                let _ = self.tx.send(frame.0, &frame.1);
                // Respect the bus budget so the kernel TX queue can drain at
                // wire rate; a tight burst of 64 frames in 2 ms (the rate
                // without pacing) loses responses on the round trip even on
                // a 250 kbit/s bus.
                std::thread::sleep(self.cfg.min_send_interval);
            }
            let deadline = Instant::now() + self.cfg.discovery_timeout;
            for w in wire..end {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if self
                    .waiter
                    .wait(&key(w), remaining.max(Duration::from_millis(1)))
                    .is_some()
                {
                    existing.push(w as u8);
                }
            }
            wire = end;
        }
        existing
    }

    /// Per-channel option-string query. Used during list/enum field discovery
    /// to resolve each option's label string id. Same channel dispatch as
    /// [`Self::meta_batch`].
    fn meta_option_req(&mut self, addr: DeviceId, fid: FieldId, opt: u8) -> Option<u16> {
        let wire = field_id::wire_index(fid);
        let (prefix, encode): (&str, MetaOptionEncode) = match field_id::channel(fid) {
            Channel::Btm1 => ("btm1_meta", btm1_meta_option_req_raw),
            Channel::Btm3 => ("btm3_meta", btm3_meta_option_req_raw),
        };
        let key = format!("{}:{:06X}:26:{}:{}", prefix, addr, wire, opt);
        self.req(&key, encode(addr, wire, opt), OPTIONAL_RETRIES)
            .filter(|r| r.len() >= 6)
            .map(|r| u16::from_le_bytes([r[4], r[5]]))
    }

    fn group_count(&mut self, addr: DeviceId, selector: u8) -> u32 {
        let key = format!("p:{:06X}:08:{:02X}", addr, selector);
        self.req_std(&key, group_count_req_raw(addr, selector))
            .filter(|r| r.len() >= 4)
            .map(|r| u16::from_le_bytes([r[2], r[3]]) as u32)
            .unwrap_or(0)
    }
}

/// Fetch a device's identity (firmware + property strings). This is the cheap
/// half of discovery — no group/field/metadata enumeration.
pub(super) fn fetch_identity(disc: &mut Disc, addr: DeviceId) -> DeviceIdentity {
    let firmware = {
        let k0 = format!("p:{:06X}:82:00", addr);
        let k1 = format!("p:{:06X}:82:01", addr);
        let r0 = disc.req_std(&k0, fw_req_raw(addr, 0)).unwrap_or_default();
        let r1 = disc.req_std(&k1, fw_req_raw(addr, 1)).unwrap_or_default();
        let major = r0.get(3).copied().unwrap_or(0) as u32 + r1.get(3).copied().unwrap_or(0) as u32;
        let minor = r0.get(2).copied().unwrap_or(0) as u32 + r1.get(2).copied().unwrap_or(0) as u32;
        format!("{}.{}", major, minor)
    };
    // Property strings are trimmed: at least one shipping charger reports its
    // article number with a trailing space ("44010250 "), which silently
    // defeats every lookup that keys on it — the bundled string catalog, and
    // any per-model mapping table. Serial is trimmed for the same reason; it
    // keys the schema cache file and the Signal K mapping.
    let fetch_prop = |disc: &mut Disc, n: u8| -> String {
        disc.prop_str_id(addr, n)
            .map(|sid| disc.fetch_str(addr, sid).trim().to_string())
            .unwrap_or_default()
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
    DeviceIdentity {
        article,
        serial,
        revision,
        name,
        firmware,
    }
}

/// Resolve this device's offline string table: pick the catalog candidate for
/// its `(article, firmware)`, confirm it against the live device by fetching a
/// few `spot_check` ids over the wire, and return the table only on a full
/// match. `None` when the model isn't bundled or no candidate spot-checks
/// clean — in which case strings are fetched live as before.
///
/// **The spot-check is the correctness guarantee.** A stale, wrong-language, or
/// wrong-revision table fails here and degrades to a live fetch, never to a
/// wrong string. See PROTOCOL §4.4 and [`crate::strings`].
pub(super) fn resolve_catalog(
    disc: &mut Disc,
    addr: DeviceId,
    id: &DeviceIdentity,
) -> Option<&'static HashMap<u16, String>> {
    for entry in crate::strings::candidates(&id.article, &id.firmware) {
        let matches = entry.spot_check.iter().all(|&sid| {
            entry
                .strings
                .get(&sid)
                .is_some_and(|want| disc.fetch_str_wire(addr, sid) == *want)
        });
        if matches {
            log::debug!(
                target: "masterbus::discovery",
                "0x{addr:06X}: string catalog hit (article {}, fw {}, {} ids, {} spot-checks)",
                id.article, entry.firmware, entry.strings.len(), entry.spot_check.len(),
            );
            return Some(&entry.strings);
        }
    }
    None
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
fn menu_range(disc: &mut Disc, addr: DeviceId, menu: Menu) -> (u32, u32) {
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
/// The cache is keyed by **serial + firmware + access level + menu**:
/// - Per-device (serial), not per-article/firmware — same-model devices
///   can differ (e.g. one battery in a cluster exposes an extra group).
/// - Per access level — writability flips per level, so a schema
///   discovered as End User can't be served as Distributor. Each level
///   gets its own cache file.
pub(super) fn discover_menu(
    disc: &mut Disc,
    addr: DeviceId,
    id: &DeviceIdentity,
    menu: Menu,
) -> Vec<GroupInfo> {
    let dir = disc.cfg.cache_path.as_deref();
    let level = disc.level;
    if let Some(g) = load_cached_menu(dir, &id.serial, &id.firmware, level, menu) {
        log::debug!(
            target: "masterbus::discovery",
            "0x{addr:06X} {menu:?}: schema loaded from cache ({} groups)",
            g.len(),
        );
        return g;
    }
    let started = std::time::Instant::now();
    log::debug!(target: "masterbus::discovery", "0x{addr:06X} {menu:?}: enumerating");
    let g = enumerate_menu(disc, addr, menu);
    log::debug!(
        target: "masterbus::discovery",
        "0x{addr:06X} {menu:?}: discovered {} groups in {:?}",
        g.len(),
        started.elapsed(),
    );
    store_cached_menu(dir, &id.serial, &id.firmware, level, menu, &g);
    g
}

fn enumerate_menu(disc: &mut Disc, addr: DeviceId, menu: Menu) -> Vec<GroupInfo> {
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

fn enumerate_group(disc: &mut Disc, addr: DeviceId, g: u8, menu: Menu) -> Option<GroupInfo> {
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

    // Field ids returned by the schema query are 8-bit wire indices on the
    // Btm1 channel; widen to channel-tagged `FieldId` with the Btm1 bit clear.
    let mut field_ids: Vec<FieldId> = Vec::new();
    for idx in 0..field_count {
        let key = format!("{}:{:06X}:03:{}:{}", key_prefix, addr, g, idx as u8);
        if let Some(r) = disc.req_std(
            &key,
            schema_field_id_req_class_raw(class, addr, g, idx as u8),
        ) && r.len() >= 6
        {
            field_ids.push(field_id::btm1(r[4]));
        }
    }

    let name = name_sid
        .map(|sid| disc.fetch_str(addr, sid))
        .unwrap_or_default();
    // Only Monitoring fields can be eventable outputs, so only they carry the
    // best-effort 0x0D query.
    let want_eventable = menu == Menu::Monitoring;
    let mut fields = Vec::new();
    for fid in field_ids {
        if let Some(f) = enumerate_field(disc, addr, fid, want_eventable) {
            fields.push(f);
        }
    }
    Some(GroupInfo {
        id: g as i32,
        name,
        menu,
        fields,
    })
}

/// Probe every reachable field across **both** metadata channels (Btm1 +
/// Btm3), dropping indices that don't respond. Returns one `FieldInfo` per
/// reachable index per channel — channels are distinguished by bit 8 of the
/// id, so callers can store the combined result in one `HashMap<FieldId, _>`.
///
/// This is the only way to enumerate Configuration items on devices whose
/// `0x08 0x03` group count lies (e.g. the Magic-class Nav Chg, which reports
/// zero config groups despite ~25 Btm3 settings). The grouping MasterAdjust
/// shows comes from hard-coded per-device-family layout
/// (`TDevice_MacMagic` etc. inside `MasterAdjust.exe`); the wire protocol
/// only exposes a flat per-channel field-index space.
pub(super) fn enumerate_all_fields(disc: &mut Disc, addr: DeviceId) -> Vec<FieldInfo> {
    // Two-phase: first an existence sweep (one VIZ query per wire index,
    // pipelined in chunks) over the full 8-bit space, then a full metadata
    // batch only for indices that responded. The previous miss-streak loop
    // was too narrow to bridge multi-index gaps (the EasyView 5 has a
    // ~0x42-index hole between Btm3 header fields and the Switch block).
    let mut out = Vec::with_capacity(64);
    probe_channel(disc, addr, Channel::Btm1, &mut out);
    probe_channel(disc, addr, Channel::Btm3, &mut out);
    out
}

/// Per-channel flat probe.
///
/// Phase 1: pipelined existence sweep ([`Disc::probe_existence`]) identifies
/// which wire indices have any field at all — one VIZ query per index, sent
/// in chunks of [`PROBE_CHUNK`] with one shared timeout per chunk.
///
/// Phase 2: full metadata batch ([`enumerate_field`], which itself pipelines
/// 5 opcodes per field) only for the indices that responded in phase 1. No
/// miss-streak — gaps are free.
fn probe_channel(disc: &mut Disc, addr: DeviceId, channel: Channel, out: &mut Vec<FieldInfo>) {
    let wires = disc.probe_existence(addr, channel, 256);
    for wire in wires {
        let fid = match channel {
            Channel::Btm1 => field_id::btm1(wire),
            Channel::Btm3 => field_id::btm3(wire),
        };
        // The flat probe is Btm3 config space; eventable outputs live on the
        // Btm1 Monitoring tab, so don't pay the 0x0D query here.
        if let Some(f) = enumerate_field(disc, addr, fid, false) {
            out.push(f);
        }
    }
}

/// How many existence-probe requests to pipeline before waiting on their
/// shared timeout. 64 keeps the response burst short enough that the
/// device's transmit queue doesn't lag behind, while still amortising the
/// per-chunk timeout (typically ~150 ms) over a meaningful slice of the
/// 256-index space.
const PROBE_CHUNK: u16 = 64;

/// Enumerate one field's metadata. `want_eventable` requests the eventable-flag
/// op (`0x0D`) as a best-effort extra — set it only for **Monitoring** fields,
/// the only tab whose fields are ever eventable outputs (confirmed 14/14 on the
/// reference bus). A device answers `0x0D` only for its eventable fields, so
/// querying it blocking on every field would pay a timeout on the silent
/// majority (the discovery slowdown this avoids).
fn enumerate_field(
    disc: &mut Disc,
    addr: DeviceId,
    fid: FieldId,
    want_eventable: bool,
) -> Option<FieldInfo> {
    use crate::protocol::meta_op as op;

    // Per-field metadata in one pipelined round-trip. Numeric editing bounds
    // (MIN/STEP) are deferred — they aren't needed to enumerate or read a field,
    // so we skip them here (op::MAX is still fetched: it doubles as the option
    // count for lists). The channel is encoded in `fid`; `meta_batch` dispatches
    // to the right encoder + waiter-key family.
    let best_effort: &[u8] = if want_eventable {
        &[op::EVENTABLE]
    } else {
        &[]
    };
    let meta = disc.meta_batch(
        addr,
        fid,
        &[op::NAME, op::VIZ, op::MAX, op::UNIT, op::WRITEABLE],
        best_effort,
    );
    // If nothing came back, this field index is unallocated on this channel —
    // used by the `enumerate_all_fields` flat probe to skip holes in the index
    // space and to give up on a channel the device doesn't speak.
    if meta.is_empty() {
        return None;
    }
    let u16_at4 = |o: u8| {
        meta.get(&o)
            .filter(|r| r.len() >= 6)
            .map(|r| u16::from_le_bytes([r[4], r[5]]))
    };
    let byte4 = |o: u8| meta.get(&o).and_then(|r| r.get(4).copied());
    let f32_at4 = |o: u8| {
        meta.get(&o)
            .filter(|r| r.len() >= 8)
            .map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]))
    };

    let name_sid = u16_at4(op::NAME);
    let viz_code = byte4(op::VIZ).unwrap_or(0x01);
    let n_or_max = f32_at4(op::MAX).unwrap_or(0.0);
    let unit_sid = u16_at4(op::UNIT);
    // Writability: meta op 0x0B, flag at byte[4].
    let writeable = byte4(op::WRITEABLE).map(|b| b != 0).unwrap_or(false);
    // Eventable-output flag: meta op 0x0D, flag at byte[4] (same shape as 0x0B).
    let eventable = byte4(op::EVENTABLE).map(|b| b != 0).unwrap_or(false);

    let viz = viz_from_wire(viz_code);
    let n_opts = n_or_max as u32;

    let options = if matches!(
        viz,
        VisualizationType::DropDown | VisualizationType::Eventable
    ) && n_opts > 0
        && n_opts <= 64
    {
        let mut opts = Vec::new();
        for opt in 0..n_opts {
            let osid = disc.meta_option_req(addr, fid, opt as u8);
            opts.push(
                osid.filter(|&s| s != 0)
                    .map(|s| disc.fetch_str(addr, s))
                    .unwrap_or_default(),
            );
        }
        opts
    } else {
        Vec::new()
    };

    let field_name = name_sid
        .filter(|&s| s != 0)
        .map(|s| disc.fetch_str(addr, s))
        .unwrap_or_default();
    let field_unit = unit_sid
        .filter(|&s| s != 0)
        .map(|s| disc.fetch_str(addr, s))
        .unwrap_or_default();

    Some(FieldInfo {
        index: fid,
        name: field_name,
        unit: field_unit,
        viz_type: viz,
        writeable,
        eventable,
        // Deferred numeric bounds (not fetched during enumeration).
        min: 0.0,
        max: n_or_max as f64,
        step: 0.0,
        options,
    })
}

// ── disk cache (groups keyed by serial + firmware + level + menu) ────────

/// Bumped whenever the cached `Vec<GroupInfo>` JSON representation changes
/// in a way that older versions can't read.
/// - v1: original (`FieldInfo.index` was `i32`).
/// - v2: `FieldInfo.index` became `u16` (`FieldId` with channel bit).
const CACHE_SCHEMA: u8 = 2;

fn cache_file(
    dir: &Path,
    serial: &str,
    firmware: &str,
    level: AccessLevel,
    menu: Menu,
) -> std::path::PathBuf {
    let key = format!(
        "{}-{}-l{:02x}-{:02x}-v{}",
        serial,
        firmware,
        level.level_byte(),
        menu.selector(),
        CACHE_SCHEMA,
    );
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{}.json", safe))
}

fn load_cached_menu(
    dir: Option<&Path>,
    serial: &str,
    firmware: &str,
    level: AccessLevel,
    menu: Menu,
) -> Option<Vec<GroupInfo>> {
    let dir = dir?;
    if serial.is_empty() {
        return None;
    }
    let data = std::fs::read(cache_file(dir, serial, firmware, level, menu)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn store_cached_menu(
    dir: Option<&Path>,
    serial: &str,
    firmware: &str,
    level: AccessLevel,
    menu: Menu,
    groups: &[GroupInfo],
) {
    let Some(dir) = dir else { return };
    if serial.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!(target: "masterbus::cache", "mkdir {}: {e}", dir.display());
        return;
    }
    let path = cache_file(dir, serial, firmware, level, menu);
    match serde_json::to_vec_pretty(groups) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!(target: "masterbus::cache", "write {}: {e}", path.display());
            } else {
                log::debug!(
                    target: "masterbus::cache",
                    "saved {} ({} groups)",
                    path.display(),
                    groups.len()
                );
            }
        }
        Err(e) => log::warn!(target: "masterbus::cache", "encode {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::error::Result;
    use crate::runtime::Config;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A `TransportTx` that answers string-chunk (`0x30`) requests from a
    /// scripted `id → full string` map by delivering the right 4-char chunk to
    /// the waiter (which `fetch_str_wire` registered just before this send).
    /// Counts sends (via a shared atomic) so a test can assert zero wire I/O on
    /// a catalog hit.
    struct ScriptTx {
        waiter: Arc<Waiter>,
        script: HashMap<u16, String>,
        sends: Arc<AtomicUsize>,
    }

    impl TransportTx for ScriptTx {
        fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
            self.sends.fetch_add(1, Ordering::Relaxed);
            if data.first() == Some(&0x30) && data.len() >= 4 {
                let sid = u16::from_le_bytes([data[1], data[2]]);
                let seq = data[3];
                let addr = can_id & 0x00FF_FFFF;
                if let Some(full) = self.script.get(&sid) {
                    let bytes = full.as_bytes();
                    let start = seq as usize * 4;
                    let mut resp = vec![0x30, data[1], data[2], seq];
                    if start >= bytes.len() {
                        resp.push(0); // terminating (empty) chunk
                    } else {
                        let end = (start + 4).min(bytes.len());
                        resp.extend_from_slice(&bytes[start..end]);
                        if end < start + 4 || end == bytes.len() {
                            resp.push(0); // NUL-terminate the final chunk
                        }
                    }
                    let key = format!("str:{:06X}:{:04X}:{}", addr, sid, seq);
                    self.waiter.deliver(&key, resp);
                }
                // Unscripted id: no delivery → the waiter times out → "".
            }
            Ok(())
        }
    }

    fn disc_with<'a>(
        tx: &'a mut ScriptTx,
        waiter: &'a Waiter,
        cfg: &'a Config,
        catalog: Option<&'static HashMap<u16, String>>,
    ) -> Disc<'a> {
        Disc {
            tx,
            waiter,
            cfg,
            level: AccessLevel::EndUser,
            catalog,
        }
    }

    #[test]
    fn cataloged_id_served_without_wire_io() {
        let waiter = Arc::new(Waiter::new());
        let cfg = Config::default();
        let table: &'static HashMap<u16, String> =
            Box::leak(Box::new(HashMap::from([(100u16, "Battery".to_string())])));
        let sends = Arc::new(AtomicUsize::new(0));
        let mut tx = ScriptTx {
            waiter: waiter.clone(),
            // id 200 is NOT in the catalog but IS scripted on the wire.
            script: HashMap::from([(200u16, "Live".to_string())]),
            sends: sends.clone(),
        };
        let mut disc = disc_with(&mut tx, &waiter, &cfg, Some(table));

        // Catalog hit: served locally, zero frames sent.
        assert_eq!(disc.fetch_str(0x0A0B0C, 100), "Battery");
        assert_eq!(sends.load(Ordering::Relaxed), 0);
        // Miss: falls through to the wire.
        assert_eq!(disc.fetch_str(0x0A0B0C, 200), "Live");
        assert!(sends.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn resolve_accepts_on_matching_spot_check_and_rejects_on_mismatch() {
        let entry = &crate::strings::candidates("77010310", "2.14")[0];
        let id = DeviceIdentity {
            article: "77010310".into(),
            serial: "X".into(),
            revision: "A".into(),
            name: "EasyView".into(),
            firmware: "2.14".into(),
        };
        let waiter = Arc::new(Waiter::new());
        let cfg = Config::default();

        // All spot-check ids answer with the catalog's own text → accept.
        let good: HashMap<u16, String> = entry
            .spot_check
            .iter()
            .map(|&s| (s, entry.strings[&s].clone()))
            .collect();
        let mut tx = ScriptTx {
            waiter: waiter.clone(),
            script: good,
            sends: Arc::new(AtomicUsize::new(0)),
        };
        let mut disc = disc_with(&mut tx, &waiter, &cfg, None);
        let resolved = resolve_catalog(&mut disc, 0x1403A4, &id).expect("should resolve");
        assert_eq!(
            resolved.get(&212).map(String::as_str),
            Some("Factory reset")
        );

        // Corrupt one spot-check answer → reject, serve live.
        let mut bad: HashMap<u16, String> = entry
            .spot_check
            .iter()
            .map(|&s| (s, entry.strings[&s].clone()))
            .collect();
        *bad.get_mut(&entry.spot_check[0]).unwrap() = "WRONG".into();
        let mut tx = ScriptTx {
            waiter: waiter.clone(),
            script: bad,
            sends: Arc::new(AtomicUsize::new(0)),
        };
        let mut disc = disc_with(&mut tx, &waiter, &cfg, None);
        assert!(resolve_catalog(&mut disc, 0x1403A4, &id).is_none());
    }
}

#[cfg(test)]
mod enumeration_tests {
    use super::*;
    use crate::protocol::VisualizationType as Viz;
    use crate::protocol::can_class;
    use crate::runtime::Engine;
    use crate::runtime::fakebus::{ADDR, Device, FakeBus};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Wire visualization codes used by the fixtures.
    const VIZ_FLOAT: u8 = 0x01;
    const VIZ_DROPDOWN: u8 = 0x03;

    fn test_config() -> Config {
        Config {
            min_send_interval: Duration::ZERO,
            discovery_timeout: Duration::from_millis(5),
            discovery_retries: 1,
            discovery_window: Duration::ZERO,
            discovery_settle: Duration::ZERO,
            connect_timeout: Duration::from_secs(5),
            cache_path: None,
            ..Config::default()
        }
    }

    fn connect(device: Device, config: Config) -> (Arc<Engine>, FakeBus) {
        let (bus, transport) = FakeBus::start(device);
        let engine = Engine::connect(transport, config).expect("connect");
        (engine, bus)
    }

    /// A device with one Monitoring group of two fields — a plain float and a
    /// two-option drop-down.
    fn combi() -> Device {
        Device::new()
            .with_identity("44010250", "1234567", "Combi")
            .with_group(Menu::Monitoring, 0, "DC", &[0x17, 0x18])
            .with_field(field_id::btm1(0x17), "Voltage", "V", VIZ_FLOAT, false)
            .with_list_field(field_id::btm1(0x18), "State", &["Off", "On"])
    }

    /// A scratch directory that cleans up after itself.
    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> TempDir {
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "masterbus-cache-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    // ── menu enumeration ────────────────────────────────────────────────────

    /// One menu, end to end: group name and field count off the schema
    /// channel, then each field's metadata and its strings.
    #[test]
    fn a_menu_is_enumerated_into_groups_and_fields() {
        let (engine, _bus) = connect(combi(), test_config());
        engine.ensure_menu(ADDR, Menu::Monitoring).unwrap();

        let schema = engine.state.schema(ADDR).unwrap();
        assert_eq!(schema.groups.len(), 1);
        let g = &schema.groups[0];
        assert_eq!((g.id, g.name.as_str(), g.menu), (0, "DC", Menu::Monitoring));
        assert_eq!(g.fields.len(), 2);

        let voltage = &g.fields[0];
        assert_eq!(voltage.index, field_id::btm1(0x17));
        assert_eq!(voltage.name, "Voltage");
        assert_eq!(voltage.unit, "V");
        assert_eq!(voltage.viz_type, Viz::Float);
        assert!(!voltage.writeable);

        let state = &g.fields[1];
        assert_eq!(state.name, "State");
        assert_eq!(state.viz_type, Viz::DropDown);
        assert!(state.writeable);
        assert_eq!(state.options, vec!["Off", "On"]);
        assert_eq!(state.max, 2.0);
    }

    /// Monitoring, Configuration and Service share one global gid space: a
    /// menu's groups start after every prior menu's. Getting the offset wrong
    /// enumerates someone else's groups.
    #[test]
    fn the_three_global_menus_share_one_offset_gid_space() {
        let device = Device::new()
            .with_identity("44010250", "1234567", "Combi")
            .with_group(Menu::Monitoring, 0, "DC", &[])
            .with_group(Menu::Monitoring, 1, "AC", &[])
            .with_group(Menu::Configuration, 2, "General", &[])
            .with_group(Menu::Service, 3, "Service", &[]);
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_menu(ADDR, Menu::Configuration).unwrap();
        engine.ensure_menu(ADDR, Menu::Service).unwrap();
        let schema = engine.state.schema(ADDR).unwrap();

        let config: Vec<_> = schema.menu_groups(Menu::Configuration).collect();
        assert_eq!(config.len(), 1);
        assert_eq!((config[0].id, config[0].name.as_str()), (2, "General"));

        let service: Vec<_> = schema.menu_groups(Menu::Service).collect();
        assert_eq!(service.len(), 1);
        assert_eq!((service[0].id, service[0].name.as_str()), (3, "Service"));
    }

    /// Alarm and History have their own gid namespaces and no reliable count
    /// query, so they're probed from 0 until two consecutive gids answer
    /// nothing.
    #[test]
    fn the_alarm_namespace_is_probed_until_two_misses() {
        let device = Device::new()
            .with_identity("44010250", "1234567", "Combi")
            .with_group(Menu::Alarm, 0, "Alarms", &[])
            .with_group(Menu::Alarm, 1, "Warnings", &[])
            // gid 2 and 3 are unallocated — the probe stops here...
            .with_group(Menu::Alarm, 4, "Never reached", &[]);
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_menu(ADDR, Menu::Alarm).unwrap();
        let schema = engine.state.schema(ADDR).unwrap();
        let names: Vec<&str> = schema
            .menu_groups(Menu::Alarm)
            .map(|g| g.name.as_str())
            .collect();
        assert_eq!(names, vec!["Alarms", "Warnings"]);
    }

    /// A device with no Alarm tab at all answers nothing and is not an error.
    #[test]
    fn a_device_without_an_alarm_tab_yields_no_groups() {
        let (engine, _bus) = connect(combi(), test_config());

        engine.ensure_menu(ADDR, Menu::Alarm).unwrap();
        assert!(engine.state.has_menu(ADDR, Menu::Alarm));
        assert_eq!(
            engine
                .state
                .schema(ADDR)
                .unwrap()
                .menu_groups(Menu::Alarm)
                .count(),
            0
        );
    }

    /// The eventable flag is asked only of Monitoring fields — the only tab
    /// whose fields are ever event targets. A field that would answer `0x0D`
    /// on another tab is never asked, so it reports as non-eventable.
    #[test]
    fn only_monitoring_fields_are_asked_for_the_eventable_flag() {
        let device = Device::new()
            .with_identity("44010250", "1234567", "Combi")
            .with_group(Menu::Monitoring, 0, "Relays", &[0x10])
            .with_group(Menu::Configuration, 1, "General", &[0x11])
            .with_eventable_field(field_id::btm1(0x10), "Relay 1")
            .with_eventable_field(field_id::btm1(0x11), "Hidden");
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_menu(ADDR, Menu::Monitoring).unwrap();
        engine.ensure_menu(ADDR, Menu::Configuration).unwrap();

        assert!(
            engine
                .state
                .field_info(ADDR, field_id::btm1(0x10))
                .unwrap()
                .eventable
        );
        assert!(
            !engine
                .state
                .field_info(ADDR, field_id::btm1(0x11))
                .unwrap()
                .eventable
        );
    }

    /// A group whose field ids the device won't hand over still appears, with
    /// no fields, rather than sinking the whole menu.
    #[test]
    fn a_group_with_unreadable_fields_is_still_reported() {
        let device = Device::new()
            .with_identity("44010250", "1234567", "Combi")
            // Field 0x17 is listed in the group but has no metadata at all.
            .with_group(Menu::Monitoring, 0, "DC", &[0x17]);
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_menu(ADDR, Menu::Monitoring).unwrap();
        let schema = engine.state.schema(ADDR).unwrap();
        assert_eq!(schema.groups.len(), 1);
        assert!(schema.groups[0].fields.is_empty());
    }

    // ── the flat probe ──────────────────────────────────────────────────────

    /// The flat probe sweeps both metadata channels; the two namespaces are
    /// independent, so the same wire index on each is a different field.
    #[test]
    fn the_flat_probe_covers_both_channels() {
        let device = Device::new()
            .with_identity("44010250", "1234567", "Nav Chg")
            .with_field(field_id::btm1(0x05), "Btm1 field", "V", VIZ_FLOAT, false)
            .with_field(field_id::btm3(0x05), "Btm3 field", "A", VIZ_FLOAT, true);
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_all_fields(ADDR).unwrap();
        let all = engine.state.all_fields(ADDR).unwrap();

        assert_eq!(all.len(), 2);
        assert_eq!(
            engine
                .state
                .field_info(ADDR, field_id::btm1(0x05))
                .unwrap()
                .name,
            "Btm1 field"
        );
        let btm3 = engine.state.field_info(ADDR, field_id::btm3(0x05)).unwrap();
        assert_eq!(btm3.name, "Btm3 field");
        assert!(btm3.writeable);
    }

    /// The probe is chunked, not miss-streak based: a wide hole in the index
    /// space (the EasyView's is ~0x42 indices) must not end the sweep.
    #[test]
    fn a_wide_hole_does_not_end_the_flat_probe() {
        let device = Device::new()
            .with_identity("77010310", "7654321", "EasyView")
            .with_field(field_id::btm3(0x00), "First", "", VIZ_FLOAT, false)
            .with_field(field_id::btm3(0xF0), "Last", "", VIZ_FLOAT, false);
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_all_fields(ADDR).unwrap();
        let names: Vec<String> = engine
            .state
            .all_fields(ADDR)
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, vec!["First", "Last"]);
    }

    /// A drop-down with an implausible option count isn't walked — a bad
    /// `0x07` read must not turn into hundreds of string fetches.
    #[test]
    fn an_absurd_option_count_is_not_walked() {
        let mut device = Device::new().with_identity("44010250", "1234567", "Combi");
        device.meta.insert(
            field_id::btm1(0x17),
            crate::runtime::fakebus::FakeField {
                name_sid: 0,
                unit_sid: 0,
                viz: VIZ_DROPDOWN,
                max: 5000.0,
                writeable: true,
                eventable: false,
                option_sids: Vec::new(),
            },
        );
        let device = device.with_group(Menu::Monitoring, 0, "DC", &[0x17]);
        let (engine, _bus) = connect(device, test_config());

        engine.ensure_menu(ADDR, Menu::Monitoring).unwrap();
        let f = engine.state.field_info(ADDR, field_id::btm1(0x17)).unwrap();
        assert_eq!(f.max, 5000.0);
        assert!(f.options.is_empty());
    }

    // ── the disk cache ──────────────────────────────────────────────────────

    /// The expensive half of discovery is cached per device: a second pass
    /// over the same menu reads the file and puts nothing on the bus.
    #[test]
    fn a_discovered_menu_is_cached_and_reused() {
        let dir = TempDir::new();
        let config = Config {
            cache_path: Some(dir.path.clone()),
            ..test_config()
        };
        let (engine, bus) = connect(combi(), config);

        engine.ensure_menu(ADDR, Menu::Monitoring).unwrap();
        let enumerated = bus.sent_class(can_class::SCHEMA_REQ).len();
        assert!(enumerated > 0, "first pass should enumerate over the wire");

        // Forget what we learned; the file should answer instead.
        engine.state.forget_schema(ADDR);
        engine.ensure_menu(ADDR, Menu::Monitoring).unwrap();

        assert_eq!(bus.sent_class(can_class::SCHEMA_REQ).len(), enumerated);
        let schema = engine.state.schema(ADDR).unwrap();
        assert_eq!(schema.groups[0].name, "DC");
        assert_eq!(schema.groups[0].fields[1].options, vec!["Off", "On"]);
    }

    /// Writability flips per access level, so each level keys its own file —
    /// as do the serial, the firmware and the menu. Sharing any of them would
    /// serve one device's (or one level's) schema as another's.
    #[test]
    fn the_cache_file_is_keyed_by_serial_firmware_level_and_menu() {
        let dir = Path::new("/cache");
        let base = cache_file(
            dir,
            "1234567",
            "1.0",
            AccessLevel::EndUser,
            Menu::Monitoring,
        );
        for other in [
            cache_file(
                dir,
                "7654321",
                "1.0",
                AccessLevel::EndUser,
                Menu::Monitoring,
            ),
            cache_file(
                dir,
                "1234567",
                "2.0",
                AccessLevel::EndUser,
                Menu::Monitoring,
            ),
            cache_file(
                dir,
                "1234567",
                "1.0",
                AccessLevel::Installer,
                Menu::Monitoring,
            ),
            cache_file(
                dir,
                "1234567",
                "1.0",
                AccessLevel::EndUser,
                Menu::Configuration,
            ),
        ] {
            assert_ne!(base, other);
        }

        // A serial read off the wire is untrusted: separators are sanitised
        // away, so the file always lands directly in the cache dir. (Dots
        // survive — firmware versions need them — but a dot alone can't
        // traverse without a separator.)
        let nasty = cache_file(
            dir,
            "../../etc/passwd",
            "1.0",
            AccessLevel::EndUser,
            Menu::Monitoring,
        );
        assert_eq!(nasty.parent(), Some(dir));
        let name = nasty.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.contains('/'), "{name}");
        assert!(name.starts_with(".._.._etc_passwd-"), "{name}");
    }

    #[test]
    fn the_cache_round_trips_groups() {
        let dir = TempDir::new();
        let groups = vec![GroupInfo {
            id: 0,
            name: "DC".into(),
            menu: Menu::Monitoring,
            fields: vec![FieldInfo {
                index: field_id::btm3(0x30),
                name: "ShutDown".into(),
                unit: "V".into(),
                viz_type: VisualizationType::Float,
                writeable: true,
                eventable: false,
                min: 0.0,
                max: 30.0,
                step: 0.0,
                options: vec![],
            }],
        }];
        let args = (
            Some(dir.path.as_path()),
            "1234567",
            "1.0",
            AccessLevel::EndUser,
            Menu::Monitoring,
        );

        assert!(load_cached_menu(args.0, args.1, args.2, args.3, args.4).is_none());
        store_cached_menu(args.0, args.1, args.2, args.3, args.4, &groups);
        assert_eq!(
            load_cached_menu(args.0, args.1, args.2, args.3, args.4).unwrap(),
            groups
        );
    }

    /// Without a serial there is no key, so nothing is written — better than
    /// every unidentified device sharing one file.
    #[test]
    fn a_device_without_a_serial_is_not_cached() {
        let dir = TempDir::new();
        store_cached_menu(
            Some(dir.path.as_path()),
            "",
            "1.0",
            AccessLevel::EndUser,
            Menu::Monitoring,
            &[],
        );
        assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 0);
        assert!(
            load_cached_menu(
                Some(dir.path.as_path()),
                "",
                "1.0",
                AccessLevel::EndUser,
                Menu::Monitoring
            )
            .is_none()
        );
    }

    /// A truncated or hand-edited cache file is ignored, not fatal: discovery
    /// falls back to the wire.
    #[test]
    fn a_corrupt_cache_file_is_ignored() {
        let dir = TempDir::new();
        let path = cache_file(
            &dir.path,
            "1234567",
            "1.0",
            AccessLevel::EndUser,
            Menu::Monitoring,
        );
        std::fs::write(&path, b"{ not json").unwrap();

        assert!(
            load_cached_menu(
                Some(dir.path.as_path()),
                "1234567",
                "1.0",
                AccessLevel::EndUser,
                Menu::Monitoring
            )
            .is_none()
        );
    }

    /// Caching off (`cache_path: None`) is a no-op in both directions.
    #[test]
    fn caching_can_be_turned_off() {
        store_cached_menu(
            None,
            "1234567",
            "1.0",
            AccessLevel::EndUser,
            Menu::Monitoring,
            &[],
        );
        assert!(
            load_cached_menu(
                None,
                "1234567",
                "1.0",
                AccessLevel::EndUser,
                Menu::Monitoring
            )
            .is_none()
        );
    }
}
