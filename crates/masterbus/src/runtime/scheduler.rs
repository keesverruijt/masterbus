//! Single bus-TX thread: discovery (high priority), on-demand reads/writes, and
//! rate-based subscription polling — paced to a bus budget, passive-first.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};

use super::discovery::{
    Disc, MENUS, discover_menu, enumerate_all_fields, fetch_identity, resolve_catalog,
};
use super::framelog::frame_log;
use super::reader::value_key;
use super::state::State;
use super::waiter::Waiter;
use super::{Command, Config, SubSpec, ValueUpdate};
use crate::error::{Error, Result};
use crate::model::{AccessLevel, Channel, DeviceId, FieldId, field_id};
use crate::model::{DeviceIdentity, Menu};
use crate::protocol::{
    TAB_DEFAULT, VisualizationType, btm3_read_raw, btm3_write_raw, decode_value, encode_commit,
    encode_login_read, encode_login_write, encode_logout, encode_set_boolean, encode_set_float,
    encode_set_list, heartbeat_raw, monitoring_req_raw, string_chunk_write_raw,
};
use crate::transport::TransportTx;
use crate::value::{Value, WriteValue};

/// How long to wait for an on-demand value response. Btm1 actively requests
/// the value and gets a fast echo on the same class. Btm3 is passive — the
/// next device-side push or write-ack delivers, and pushes are observed at
/// ~2 s intervals on the reference bus, so a single read after discovery
/// can wait that long.
const VALUE_READ_TIMEOUT_BTM1: Duration = Duration::from_millis(500);
const VALUE_READ_TIMEOUT_BTM3: Duration = Duration::from_millis(2500);

struct SubState {
    spec: SubSpec,
    next_due: HashMap<FieldId, Instant>,
    last_value: HashMap<FieldId, Value>,
}

pub(super) fn spawn(
    tx: Box<dyn TransportTx>,
    state: Arc<State>,
    waiter: Arc<Waiter>,
    cmd_rx: Receiver<Command>,
    shutdown: Arc<AtomicBool>,
    config: Config,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("masterbus-scheduler".into())
        .spawn(move || {
            // Fire the first heartbeat immediately (if configured) to prompt
            // device announcements as soon as we connect.
            let next_heartbeat = config.heartbeat_master.map(|_| Instant::now());
            Sched {
                tx,
                state,
                waiter,
                config,
                last_send: Instant::now(),
                next_heartbeat,
            }
            .run(cmd_rx, &shutdown)
        })
        .expect("spawn scheduler")
}

struct Sched {
    tx: Box<dyn TransportTx>,
    state: Arc<State>,
    waiter: Arc<Waiter>,
    config: Config,
    last_send: Instant,
    next_heartbeat: Option<Instant>,
}

impl Sched {
    fn run(mut self, cmd_rx: Receiver<Command>, shutdown: &AtomicBool) {
        let mut subs: Vec<SubState> = Vec::new();
        while !shutdown.load(Ordering::Relaxed) {
            let timeout = self.loop_timeout(&subs);
            match cmd_rx.recv_timeout(timeout) {
                Ok(Command::Identify { addr, reply }) => {
                    self.do_identify(addr);
                    let _ = reply.send(Ok(()));
                }
                Ok(Command::DiscoverMenu { addr, menu, reply }) => {
                    self.do_discover_menu(addr, menu);
                    let _ = reply.send(Ok(()));
                }
                Ok(Command::Discover { addr, reply }) => {
                    self.do_discover_all(addr);
                    let _ = reply.send(Ok(()));
                }
                Ok(Command::DiscoverAllFields { addr, reply }) => {
                    self.do_discover_all_fields(addr);
                    let _ = reply.send(Ok(()));
                }
                Ok(Command::Read {
                    addr,
                    field,
                    max_age,
                    reply,
                }) => {
                    let r = self.do_read(addr, field, max_age);
                    let _ = reply.send(r);
                }
                Ok(Command::Write {
                    addr,
                    field,
                    value,
                    reply,
                }) => {
                    let r = self.do_write(addr, field, value);
                    let _ = reply.send(r);
                }
                Ok(Command::WriteString {
                    addr,
                    str_id,
                    text,
                    reply,
                }) => {
                    let r = self.do_write_string(addr, str_id, &text);
                    let _ = reply.send(r);
                }
                Ok(Command::AccessLevelRead { addr, reply }) => {
                    let r = self.do_access_level_read(addr);
                    let _ = reply.send(r);
                }
                Ok(Command::AccessLevelSet {
                    addr,
                    level,
                    code,
                    reply,
                }) => {
                    let r = self.do_access_level_set(addr, level, code);
                    let _ = reply.send(r);
                }
                Ok(Command::Subscribe(spec)) => self.add_sub(&mut subs, spec),
                Ok(Command::Unsubscribe(id)) => subs.retain(|s| s.spec.id != id),
                Ok(Command::Shutdown) => break,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.maybe_heartbeat();
            self.process_due(&mut subs);
        }
        self.waiter.cancel_all();
    }

    /// How long to block on the command channel: the soonest of the next due
    /// poll, the next heartbeat, and a 200 ms idle tick.
    fn loop_timeout(&self, subs: &[SubState]) -> Duration {
        let mut t = self.next_due_in(subs).unwrap_or(Duration::from_millis(200));
        if let Some(hb) = self.next_heartbeat {
            t = t.min(hb.saturating_duration_since(Instant::now()));
        }
        t
    }

    /// Emit the bus-master heartbeat if it's due (no-op unless configured).
    fn maybe_heartbeat(&mut self) {
        let Some(master) = self.config.heartbeat_master else {
            return;
        };
        let now = Instant::now();
        if self.next_heartbeat.is_some_and(|t| now >= t) {
            self.send(heartbeat_raw(master));
            self.next_heartbeat = Some(now + self.config.heartbeat_interval);
        }
    }

    /// Respect the bus budget: ensure `min_send_interval` between transmissions.
    fn pace(&mut self) {
        let since = self.last_send.elapsed();
        if since < self.config.min_send_interval {
            std::thread::sleep(self.config.min_send_interval - since);
        }
        self.last_send = Instant::now();
    }

    fn send(&mut self, frame: (u32, Vec<u8>)) {
        self.pace();
        frame_log("Tx", frame.0, &frame.1);
        let _ = self.tx.send(frame.0, &frame.1);
    }

    /// Fetch (and cache) just a device's identity — the cheap half of discovery.
    fn do_identify(&mut self, addr: DeviceId) {
        if self.state.has_identity(addr) {
            return;
        }
        let id = self.with_disc(addr, |disc| fetch_identity(disc, addr));
        self.state.put_identity(addr, id);
    }

    /// Ensure the identity is known (reusing the cache), returning it.
    fn ensure_identity(&mut self, addr: DeviceId) -> DeviceIdentity {
        if let Some(id) = self.state.identity(addr) {
            return id;
        }
        let id = self.with_disc(addr, |disc| fetch_identity(disc, addr));
        self.state.put_identity(addr, id.clone());
        id
    }

    /// Resolve (once) the device's offline string table via a live spot-check,
    /// so subsequent string fetches during enumeration can be served locally.
    /// Cheap and idempotent: a few round trips on first call, nothing after.
    /// A miss (unbundled model or failed spot-check) is remembered too, so we
    /// don't re-probe — strings then fetch live exactly as before.
    fn ensure_catalog(&mut self, addr: DeviceId, id: &DeviceIdentity) {
        if self.state.catalog_attempted(addr) {
            return;
        }
        let table = self.with_disc(addr, |disc| resolve_catalog(disc, addr, id));
        self.state.put_catalog(addr, table);
    }

    /// Discover one menu's groups for a device (the lazy unit of discovery).
    fn do_discover_menu(&mut self, addr: DeviceId, menu: Menu) {
        if self.state.has_menu(addr, menu) {
            return;
        }
        let id = self.ensure_identity(addr);
        self.ensure_catalog(addr, &id);
        // Discover this device's own menu (per-device disk cache inside). We do
        // not reuse another same-article device's schema: devices that share an
        // article can differ (e.g. the cluster master battery has an extra group).
        let groups = self.with_disc(addr, |disc| discover_menu(disc, addr, &id, menu));
        self.last_send = Instant::now();
        self.state.put_menu(addr, menu, groups);
    }

    /// Discover every menu (a full schema).
    fn do_discover_all(&mut self, addr: DeviceId) {
        for menu in MENUS {
            self.do_discover_menu(addr, menu);
        }
    }

    /// Discover every field via the flat index-space probe (selector `0x01`).
    fn do_discover_all_fields(&mut self, addr: DeviceId) {
        if self.state.has_all_fields(addr) {
            return;
        }
        let id = self.ensure_identity(addr);
        self.ensure_catalog(addr, &id);
        let fields = self.with_disc(addr, |disc| enumerate_all_fields(disc, addr));
        self.last_send = Instant::now();
        self.state.put_all_fields(addr, fields);
    }

    /// Run a closure with a [`Disc`] bound to this scheduler's transport.
    /// The `addr` argument lets us pre-populate the device's current access
    /// level — discovery uses it as part of the disk-cache key so each
    /// level gets its own cached schema. On first contact we ask the
    /// device (`0x08 0x19` read); if that times out we settle on
    /// `EndUser` (the device's boot state) and don't re-ask.
    fn with_disc<T>(&mut self, addr: DeviceId, f: impl FnOnce(&mut Disc) -> T) -> T {
        let level = match self.state.access_level(addr) {
            Some(l) => l,
            None => {
                let l = self
                    .do_access_level_read(addr)
                    .unwrap_or(AccessLevel::EndUser);
                self.state.put_access_level(addr, l);
                l
            }
        };
        let waiter = self.waiter.clone();
        let cfg = self.config.clone();
        // Inject the device's spot-checked string table (if resolved). `None`
        // until `ensure_catalog` has run, so identity discovery stays live.
        let catalog = self.state.catalog_table(addr);
        let mut disc = Disc {
            tx: &mut *self.tx,
            waiter: &waiter,
            cfg: &cfg,
            level,
            catalog,
        };
        f(&mut disc)
    }

    fn viz_of(&mut self, addr: DeviceId, field: FieldId) -> Result<VisualizationType> {
        // Look in both the menu-grouped schema (Btm1) and the flat probe list
        // (Btm3). Without the all_fields path, Btm3-channel fields always
        // returned `FieldNotAvailable` here, which silently skipped the
        // value-read in `poll_value` — that's why Btm3 fields never showed a
        // value even after a Settings-tab probe.
        if let Some(f) = self.state.field_info(addr, field) {
            return Ok(f.viz_type);
        }
        // Field still unknown: trigger the right discovery for its channel.
        match field_id::channel(field) {
            Channel::Btm1 => self.do_discover_all(addr),
            Channel::Btm3 => self.do_discover_all_fields(addr),
        }
        self.state
            .field_info(addr, field)
            .map(|f| f.viz_type)
            .ok_or(Error::FieldNotAvailable(field as i32))
    }

    fn poll_value(
        &mut self,
        addr: DeviceId,
        field: FieldId,
        viz: VisualizationType,
    ) -> Result<Value> {
        let key = value_key(addr, field);
        self.waiter.register(&key);
        // Both channels require an active read request. Btm3 value pushes are
        // NOT autonomous — the device only emits a class-`0x0B` frame in
        // response to an explicit class-`0x1B` read; on a quiet bus (no other
        // master polling) we'd never see anything otherwise.
        let timeout = match field_id::channel(field) {
            Channel::Btm1 => {
                self.send(monitoring_req_raw(
                    addr,
                    field_id::wire_index(field),
                    TAB_DEFAULT,
                ));
                VALUE_READ_TIMEOUT_BTM1
            }
            Channel::Btm3 => {
                self.send(btm3_read_raw(addr, field_id::wire_index(field)));
                VALUE_READ_TIMEOUT_BTM3
            }
        };
        match self.waiter.wait(&key, timeout) {
            Some(raw) => {
                let opts = self
                    .state
                    .schema(addr)
                    .and_then(|s| s.field(field).map(|f| f.options.clone()))
                    .unwrap_or_default();
                let mut v = decode_value(&raw, viz).with_options(&opts);
                // For Text-VIZ fields the value is the *sid* of the editable
                // content; fetch the actual chars from the string table.
                if let Value::Text { sid, ref mut text } = v {
                    *text = self.with_disc(addr, |disc| disc.fetch_str(addr, sid));
                }
                self.state.put_value(addr, field, v.clone());
                Ok(v)
            }
            None => Err(Error::Timeout),
        }
    }

    fn do_read(&mut self, addr: DeviceId, field: FieldId, max_age: Duration) -> Result<Value> {
        if let Some(cv) = self.state.get_value(addr, field)
            && !cv.outdated
            && cv.at.elapsed() <= max_age
        {
            return Ok(cv.value);
        }
        let viz = self.viz_of(addr, field)?;
        self.poll_value(addr, field, viz)
    }

    // ── access-level login (opcode 0x08 0x19 on class 0x07) ────────────────
    //
    // Read, login, and logout all share the same waiter key `p:<addr>:08:19`
    // because the response is a class-0x06 frame `[0x08, 0x19, level, 0x00]`
    // in all three cases. The level byte at data[2] is what we return.

    fn await_access_level(&mut self, addr: DeviceId) -> Result<AccessLevel> {
        let key = format!("p:{:06X}:08:19", addr);
        // The login response shares timing with a Btm1 value-read echo —
        // both arrive in the tens of ms.
        match self.waiter.wait(&key, VALUE_READ_TIMEOUT_BTM1) {
            Some(data) if data.len() >= 3 => AccessLevel::from_byte(data[2]).ok_or_else(|| {
                Error::Protocol(format!("unknown access level byte 0x{:02X}", data[2]))
            }),
            Some(_) => Err(Error::Protocol("short access-level response".into())),
            None => Err(Error::Timeout),
        }
    }

    fn do_access_level_read(&mut self, addr: DeviceId) -> Result<AccessLevel> {
        let key = format!("p:{:06X}:08:19", addr);
        self.waiter.register(&key);
        self.send(encode_login_read(addr));
        let level = self.await_access_level(addr)?;
        self.state.put_access_level(addr, level);
        Ok(level)
    }

    fn do_access_level_set(
        &mut self,
        addr: DeviceId,
        level: AccessLevel,
        code: Option<f32>,
    ) -> Result<AccessLevel> {
        let key = format!("p:{:06X}:08:19", addr);
        self.waiter.register(&key);
        let frame = match code {
            Some(c) => encode_login_write(addr, level.level_byte(), c),
            None => encode_logout(addr),
        };
        self.send(frame);
        let reported = self.await_access_level(addr)?;
        // A level change flips per-field WRITEABLE (meta op 0x0B) on many
        // fields; the in-memory schema cached at the prior level is now
        // stale. Drop it so the next field access reloads from the disk
        // cache (now keyed by access level) or re-runs discovery.
        // PROTOCOL.md §4.5.
        self.state.put_access_level(addr, reported);
        self.state.forget_schema(addr);
        Ok(reported)
    }

    fn do_write(&mut self, addr: DeviceId, field: FieldId, value: WriteValue) -> Result<Value> {
        log::info!(
            target: "masterbus::write",
            "→ write 0x{addr:06X} field 0x{field:04X} = {value:?}",
        );
        // Text fields write via the string-chunk protocol (PROTOCOL.md §4.4),
        // not the numeric-write path: the field's "value" is the sid, the
        // editable content lives at that sid in the string table.
        if let WriteValue::Text { sid, text } = value {
            self.do_write_string(addr, sid, &text)?;
            let v = Value::Text { sid, text };
            self.state.put_value(addr, field, v.clone());
            return Ok(v);
        }
        let wire = field_id::wire_index(field);
        match field_id::channel(field) {
            Channel::Btm1 => {
                // Relay-style boolean controls (e.g. the CombiMaster inverter
                // / charger) only act when the value write is followed by a
                // fixed "commit" token to the adjacent hidden command
                // register at field+1. The CombiMaster reports that register
                // in schema discovery as an unnamed FieldInfo (empty name,
                // zero min/max/step, no options) — treat empty-name as
                // hidden. If a *named* user-facing field is in the way,
                // refuse the write rather than silently failing to actuate.
                if matches!(value, WriteValue::Bool(_)) {
                    let cmd = field + 1;
                    let cmd_field = self.state.schema(addr).and_then(|s| s.field(cmd).cloned());
                    if let Some(f) = &cmd_field {
                        if !f.name.is_empty() {
                            log::warn!(
                                target: "masterbus::write",
                                "  refusing boolean write: commit slot at fid 0x{cmd:04X} \
                                 is occupied by named field \"{}\". schema entry: {f:?}",
                                f.name,
                            );
                            return Err(Error::CommitFieldOccupied {
                                field: field as i32,
                                cmd_field: cmd as i32,
                                cmd_field_name: f.name.clone(),
                            });
                        }
                    }
                }
                let frame = match value {
                    WriteValue::Bool(b) => encode_set_boolean(addr, wire, b),
                    WriteValue::Float(f) => encode_set_float(addr, wire, f),
                    WriteValue::ListIndex(i) => encode_set_list(addr, wire, i),
                    WriteValue::Text { .. } => unreachable!("handled above"),
                };
                self.send(frame);
                if matches!(value, WriteValue::Bool(_)) {
                    let cmd = field + 1;
                    log::info!(
                        target: "masterbus::write",
                        "  emitting commit token at fid 0x{cmd:04X}",
                    );
                    self.send(encode_commit(addr, field_id::wire_index(cmd)));
                }
            }
            Channel::Btm3 => {
                // Every Btm3 write is a 4-byte f32, regardless of viz type:
                // booleans go as 1.0 / 0.0, list picks go as the index as
                // f32 (PROTOCOL.md §6, observed live in FINDINGS §3e).
                let v = match value {
                    WriteValue::Bool(true) => 1.0,
                    WriteValue::Bool(false) => 0.0,
                    WriteValue::Float(f) => f,
                    WriteValue::ListIndex(i) => i as f32,
                    WriteValue::Text { .. } => unreachable!("handled above"),
                };
                self.send(btm3_write_raw(addr, wire, v));
            }
        }
        self.state.mark_outdated(addr, field);
        // Confirm by observing the resulting value. On Btm1 the echo on class
        // 0x08 lands within tens of ms; on Btm3 the ack on class 0x0B lands
        // within ~10 ms.
        let viz = self.viz_of(addr, field)?;
        let result = self.poll_value(addr, field, viz);
        match &result {
            Ok(v) => log::info!(
                target: "masterbus::write",
                "← read-back 0x{addr:06X} field 0x{field:04X} = {v:?}",
            ),
            Err(e) => log::info!(
                target: "masterbus::write",
                "← read-back 0x{addr:06X} field 0x{field:04X} failed: {e}",
            ),
        }
        result
    }

    /// Write the device's string table at `str_id` (PROTOCOL.md §4.4 write
    /// direction): send up-to-4-char chunks `[0x30, sid_lo, sid_hi, seq, c0..]`
    /// on class `0x07`, wait for each class-`0x06` echo, then a NUL-terminator
    /// chunk. Even when the string is exactly N×4 chars MasterAdjust emits an
    /// explicit `[0x30, sid_lo, sid_hi, N, 0x00]` terminator, so we do too.
    fn do_write_string(&mut self, addr: DeviceId, str_id: u16, text: &str) -> Result<()> {
        let bytes = text.as_bytes();
        for (seq, chunk) in bytes.chunks(4).enumerate() {
            self.send_string_chunk(addr, str_id, seq as u8, chunk)?;
        }
        // Explicit terminator chunk — sequence number is the chunk *after* the
        // last full chunk, payload is a single 0x00.
        let term_seq = bytes.len().div_ceil(4) as u8;
        self.send_string_chunk(addr, str_id, term_seq, &[0x00])?;
        Ok(())
    }

    /// Send one chunk and wait for the device's echo ack on class `0x06`.
    /// The echo arrives via the existing `str:<addr>:<sid>:<seq>` waiter key
    /// (see `protocol::decode::waiter_key_for_frame`).
    fn send_string_chunk(
        &mut self,
        addr: DeviceId,
        str_id: u16,
        seq: u8,
        chars: &[u8],
    ) -> Result<()> {
        let key = format!("str:{:06X}:{:04X}:{}", addr, str_id, seq);
        self.waiter.register(&key);
        self.send(string_chunk_write_raw(addr, str_id, seq, chars));
        match self.waiter.wait(&key, VALUE_READ_TIMEOUT_BTM1) {
            Some(_) => Ok(()),
            None => Err(Error::Timeout),
        }
    }

    // ── subscriptions ───────────────────────────────────────────────────────

    fn add_sub(&mut self, subs: &mut Vec<SubState>, spec: SubSpec) {
        // Ensure the subscribed fields' types are known. We don't know which menus
        // they live in, so discover the full schema (one-time; disk-cached).
        if !spec
            .fields
            .iter()
            .all(|&f| self.state.has_field(spec.device, f))
        {
            self.do_discover_all(spec.device);
        }
        let now = Instant::now();
        let next_due = spec.fields.iter().map(|&f| (f, now)).collect();
        subs.push(SubState {
            spec,
            next_due,
            last_value: HashMap::new(),
        });
    }

    fn next_due_in(&self, subs: &[SubState]) -> Option<Duration> {
        let now = Instant::now();
        subs.iter()
            .flat_map(|s| s.next_due.values())
            .map(|&due| due.saturating_duration_since(now))
            .min()
    }

    fn process_due(&mut self, subs: &mut [SubState]) {
        let now = Instant::now();
        // Collect work first to avoid borrow issues.
        let mut work: Vec<(usize, FieldId)> = Vec::new();
        for (i, s) in subs.iter().enumerate() {
            for (&f, &due) in &s.next_due {
                if due <= now {
                    work.push((i, f));
                }
            }
        }
        for (i, field) in work {
            let (device, interval, change_only) = {
                let s = &subs[i];
                (s.spec.device, s.spec.interval, s.spec.change_only)
            };
            // passive-first: use a cache value fresh within the interval, else poll.
            let value = match self.state.get_value(device, field) {
                Some(cv) if !cv.outdated && cv.at.elapsed() <= interval => Some(cv.value),
                _ => match self.viz_of(device, field) {
                    Ok(viz) => self.poll_value(device, field, viz).ok(),
                    Err(_) => None,
                },
            };
            let s = &mut subs[i];
            s.next_due.insert(field, now + interval);
            if let Some(v) = value {
                let changed = s.last_value.get(&field) != Some(&v);
                if !change_only || changed {
                    s.last_value.insert(field, v.clone());
                    let _ = s.spec.sender.send(ValueUpdate {
                        device,
                        field,
                        value: v,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceIdentity, FieldInfo, GroupInfo};
    use crate::protocol::can_class;
    use crate::runtime::Engine;
    use crate::runtime::fakebus::{ADDR, Device, FakeBus};

    /// Config for a test bus: no pacing, short discovery timeouts (nothing
    /// here waits out a real device), no disk cache.
    fn test_config() -> Config {
        Config {
            min_send_interval: Duration::ZERO,
            discovery_timeout: Duration::from_millis(5),
            discovery_retries: 1,
            discovery_window: Duration::ZERO,
            discovery_settle: Duration::ZERO,
            connect_timeout: Duration::from_secs(5),
            liveness: Duration::from_millis(200),
            cache_path: None,
            ..Config::default()
        }
    }

    fn connect(device: Device, config: Config) -> (Arc<Engine>, FakeBus) {
        let (bus, transport) = FakeBus::start(device);
        let engine = Engine::connect(transport, config).expect("connect");
        (engine, bus)
    }

    fn field(index: FieldId, name: &str, viz: VisualizationType) -> FieldInfo {
        FieldInfo {
            index,
            name: name.to_string(),
            unit: String::new(),
            viz_type: viz,
            writeable: true,
            eventable: false,
            min: 0.0,
            max: 0.0,
            step: 0.0,
            options: Vec::new(),
        }
    }

    /// Put a discovered schema in place, so the scheduler's read/write paths
    /// run without first enumerating the device.
    fn seed_schema(engine: &Engine, fields: Vec<FieldInfo>) {
        engine.state.put_identity(
            ADDR,
            DeviceIdentity {
                article: "44010250".into(),
                serial: "1234567".into(),
                revision: "3".into(),
                name: "Combi".into(),
                firmware: "1.0".into(),
            },
        );
        let (btm1, btm3): (Vec<_>, Vec<_>) = fields
            .into_iter()
            .partition(|f| field_id::channel(f.index) == Channel::Btm1);
        engine.state.put_menu(
            ADDR,
            Menu::Monitoring,
            vec![GroupInfo {
                id: 0,
                name: "Monitoring".into(),
                menu: Menu::Monitoring,
                fields: btm1,
            }],
        );
        if !btm3.is_empty() {
            engine.state.put_all_fields(ADDR, btm3);
        }
    }

    // ── values ──────────────────────────────────────────────────────────────

    /// A read with no usable cache entry goes out on the wire as a class-0x18
    /// request and comes back decoded per the field's visualization type.
    #[test]
    fn an_on_demand_read_polls_the_device() {
        let device = Device::new().with_btm1(0x17, 12.5);
        let (engine, bus) = connect(device, test_config());
        let f = field_id::btm1(0x17);
        seed_schema(&engine, vec![field(f, "Voltage", VisualizationType::Float)]);

        assert_eq!(
            engine.read(ADDR, f, Duration::ZERO).unwrap(),
            Value::Float(12.5)
        );
        assert_eq!(
            bus.sent_class(can_class::MONITORING_REQ),
            vec![(0x18_188EA2, vec![0x17, TAB_DEFAULT])]
        );
    }

    /// The point of the value cache: a read inside `max_age` is answered from
    /// memory and puts nothing on the bus.
    #[test]
    fn a_fresh_cached_value_is_served_without_touching_the_bus() {
        let device = Device::new().with_btm1(0x17, 12.5);
        let (engine, bus) = connect(device, test_config());
        let f = field_id::btm1(0x17);
        seed_schema(&engine, vec![field(f, "Voltage", VisualizationType::Float)]);

        engine.read(ADDR, f, Duration::ZERO).unwrap();
        let after_first = bus.sent_class(can_class::MONITORING_REQ).len();

        // Served from cache…
        assert_eq!(
            engine.read(ADDR, f, Duration::from_secs(60)).unwrap(),
            Value::Float(12.5)
        );
        assert_eq!(bus.sent_class(can_class::MONITORING_REQ).len(), after_first);

        // …until the caller demands something fresher.
        engine.read(ADDR, f, Duration::ZERO).unwrap();
        assert_eq!(
            bus.sent_class(can_class::MONITORING_REQ).len(),
            after_first + 1
        );
    }

    /// A field the device never answers for resolves as a timeout rather than
    /// hanging the caller.
    #[test]
    fn a_silent_field_times_out() {
        let mut device = Device::new();
        device.mute.push(0x17);
        let (engine, _bus) = connect(device, test_config());
        let f = field_id::btm1(0x17);
        seed_schema(&engine, vec![field(f, "Voltage", VisualizationType::Float)]);

        assert!(matches!(
            engine.read(ADDR, f, Duration::ZERO),
            Err(Error::Timeout)
        ));
    }

    /// A field that is in neither the schema nor the flat probe list sends the
    /// scheduler into discovery; when the device won't enumerate either, the
    /// read reports the field as unavailable rather than timing out.
    #[test]
    fn an_undiscoverable_field_is_reported_unavailable() {
        let (engine, _bus) = connect(Device::new(), test_config());
        let f = field_id::btm1(0x17);

        assert!(matches!(
            engine.read(ADDR, f, Duration::ZERO),
            Err(Error::FieldNotAvailable(0x17))
        ));
    }

    /// Btm3 values live on the shadow address (`addr | 0x800000`) with a
    /// headerless payload, and decode through the flat probe list.
    #[test]
    fn a_btm3_read_uses_the_shadow_address() {
        let device = Device::new().with_btm3(0x30, 17.0);
        let (engine, bus) = connect(device, test_config());
        let f = field_id::btm3(0x30);
        seed_schema(
            &engine,
            vec![field(f, "ShutDown", VisualizationType::Float)],
        );

        assert_eq!(
            engine.read(ADDR, f, Duration::ZERO).unwrap(),
            Value::Float(17.0)
        );
        assert_eq!(
            bus.sent_class(can_class::SCHEMA_REQ_HISTORY),
            vec![(0x1B_988EA2, vec![0x30, 0x00])]
        );
    }

    // ── writes ──────────────────────────────────────────────────────────────

    #[test]
    fn a_float_write_lands_and_is_read_back() {
        let device = Device::new().with_btm1(0x30, 14.4);
        let (engine, bus) = connect(device, test_config());
        let f = field_id::btm1(0x30);
        seed_schema(
            &engine,
            vec![field(f, "Absorption", VisualizationType::Float)],
        );

        assert_eq!(
            engine.write(ADDR, f, WriteValue::Float(15.2)).unwrap(),
            Value::Float(15.2)
        );
        assert_eq!(
            bus.device.lock().unwrap().btm1[&0x30],
            15.2f32.to_le_bytes()
        );
    }

    /// Relay-style booleans only actuate when the value write is followed by
    /// the commit token at the adjacent hidden register.
    #[test]
    fn a_boolean_write_is_followed_by_the_commit_token() {
        let (engine, bus) = connect(Device::new(), test_config());
        let f = field_id::btm1(0x13);
        seed_schema(
            &engine,
            vec![
                field(f, "Inverter", VisualizationType::CheckBox),
                // The CombiMaster reports the command register as an unnamed
                // field — that's what marks the slot as free to commit into.
                field(f + 1, "", VisualizationType::Float),
            ],
        );

        assert_eq!(
            engine.write(ADDR, f, WriteValue::Bool(true)).unwrap(),
            Value::Boolean(true)
        );

        let writes: Vec<Vec<u8>> = bus
            .sent_class(can_class::MONITORING_REQ)
            .into_iter()
            .map(|(_, d)| d)
            .filter(|d| d.len() == 6)
            .collect();
        assert_eq!(
            writes,
            vec![
                vec![0x13, 0x00, 0x00, 0x00, 0x80, 0x3F],
                // The commit token itself, captured from MasterAdjust.
                vec![0x14, 0x00, 0x14, 0x9F, 0x3C, 0x02],
            ]
        );
    }

    /// If a *named*, user-facing field occupies the commit slot, the write is
    /// refused: committing there would clobber it, and not committing would
    /// silently fail to actuate.
    #[test]
    fn a_boolean_write_is_refused_when_the_commit_slot_is_occupied() {
        let (engine, bus) = connect(Device::new(), test_config());
        let f = field_id::btm1(0x13);
        seed_schema(
            &engine,
            vec![
                field(f, "Inverter", VisualizationType::CheckBox),
                field(f + 1, "Charger", VisualizationType::CheckBox),
            ],
        );

        let err = engine.write(ADDR, f, WriteValue::Bool(true)).unwrap_err();
        assert!(matches!(
            err,
            Error::CommitFieldOccupied {
                field: 0x13,
                cmd_field: 0x14,
                ref cmd_field_name,
            } if cmd_field_name == "Charger"
        ));
        // Nothing went out: the refusal happens before the value write.
        assert!(bus.sent_class(can_class::MONITORING_REQ).is_empty());
    }

    /// Every Btm3 write is a 4-byte f32 — a list pick goes as its index — and
    /// the device's ack on the value carrier confirms it.
    #[test]
    fn a_btm3_list_write_goes_out_as_a_float() {
        let (engine, bus) = connect(Device::new(), test_config());
        let f = field_id::btm3(0x2C);
        seed_schema(&engine, vec![field(f, "Mode", VisualizationType::DropDown)]);

        assert_eq!(
            engine.write(ADDR, f, WriteValue::ListIndex(2)).unwrap(),
            Value::List {
                index: 2,
                options: Vec::new()
            }
        );
        assert_eq!(
            bus.sent_class(can_class::SCHEMA_REQ_HISTORY)
                .first()
                .unwrap(),
            &(0x1B_988EA2, vec![0x2C, 0x00, 0x00, 0x00, 0x00, 0x40])
        );
    }

    /// A Text write goes through the string-chunk protocol: four chars per
    /// frame, then an explicit NUL terminator — never the numeric write path.
    #[test]
    fn a_text_write_is_chunked_and_terminated() {
        let (engine, bus) = connect(Device::new(), test_config());

        engine.write_string(ADDR, 0x0001, "NavigationCh").unwrap();

        let chunks: Vec<Vec<u8>> = bus
            .sent_class(can_class::PROPERTY_REQ)
            .into_iter()
            .map(|(_, d)| d)
            .collect();
        assert_eq!(
            chunks,
            vec![
                vec![0x30, 0x01, 0x00, 0x00, b'N', b'a', b'v', b'i'],
                vec![0x30, 0x01, 0x00, 0x01, b'g', b'a', b't', b'i'],
                vec![0x30, 0x01, 0x00, 0x02, b'o', b'n', b'C', b'h'],
                vec![0x30, 0x01, 0x00, 0x03, 0x00],
            ]
        );
        assert_eq!(bus.device.lock().unwrap().strings[&0x0001], "NavigationCh");
    }

    /// Writing a Text *field* takes the same path and caches the new text
    /// against the field, without a numeric read-back.
    #[test]
    fn a_text_field_write_caches_the_new_text() {
        let (engine, _bus) = connect(Device::new(), test_config());
        let f = field_id::btm1(0x01);
        seed_schema(
            &engine,
            vec![field(f, "Device name", VisualizationType::Text)],
        );

        let written = engine
            .write(
                ADDR,
                f,
                WriteValue::Text {
                    sid: 0x0001,
                    text: "Combi".into(),
                },
            )
            .unwrap();
        assert_eq!(
            written,
            Value::Text {
                sid: 0x0001,
                text: "Combi".into()
            }
        );
        assert_eq!(engine.state.get_value(ADDR, f).unwrap().value, written);
    }

    // ── access level ────────────────────────────────────────────────────────

    #[test]
    fn the_access_level_is_read_from_the_device() {
        let mut device = Device::new();
        device.level = AccessLevel::Installer;
        let (engine, bus) = connect(device, test_config());

        assert_eq!(engine.access_level(ADDR).unwrap(), AccessLevel::Installer);
        assert_eq!(
            engine.state.access_level(ADDR),
            Some(AccessLevel::Installer)
        );
        assert_eq!(
            bus.sent_class(can_class::PROPERTY_REQ),
            vec![(0x07_188EA2, vec![0x08, 0x19])]
        );
    }

    /// A successful login raises the level *and* drops the cached schema:
    /// writability flips per level, so the discovered attributes are stale.
    #[test]
    fn a_login_raises_the_level_and_forgets_the_schema() {
        let mut device = Device::new();
        device.codes.insert(1, 1234.0);
        let (engine, _bus) = connect(device, test_config());
        seed_schema(
            &engine,
            vec![field(
                field_id::btm1(0x17),
                "Voltage",
                VisualizationType::Float,
            )],
        );
        assert!(engine.state.schema(ADDR).is_some());

        let level = engine
            .set_access_level(ADDR, AccessLevel::Installer, Some(1234.0))
            .unwrap();
        assert_eq!(level, AccessLevel::Installer);
        assert_eq!(
            engine.state.access_level(ADDR),
            Some(AccessLevel::Installer)
        );
        assert!(engine.state.schema(ADDR).is_none());
    }

    /// A wrong code isn't an error on the wire — the device just answers with
    /// the level you were already at. Callers detect it by comparing.
    #[test]
    fn a_wrong_login_code_leaves_the_level_alone() {
        let mut device = Device::new();
        device.codes.insert(1, 1234.0);
        let (engine, _bus) = connect(device, test_config());

        let level = engine
            .set_access_level(ADDR, AccessLevel::Installer, Some(9999.0))
            .unwrap();
        assert_eq!(level, AccessLevel::EndUser);
    }

    #[test]
    fn a_logout_returns_to_end_user() {
        let mut device = Device::new();
        device.level = AccessLevel::Distributor;
        let (engine, bus) = connect(device, test_config());

        assert_eq!(
            engine
                .set_access_level(ADDR, AccessLevel::EndUser, None)
                .unwrap(),
            AccessLevel::EndUser
        );
        assert_eq!(bus.device.lock().unwrap().level, AccessLevel::EndUser);
    }

    // ── identity ────────────────────────────────────────────────────────────

    /// Identity is the cheap half of discovery: firmware halves, four property
    /// string ids, and the strings they point at. It is fetched once.
    #[test]
    fn the_identity_is_fetched_once_and_then_served_from_memory() {
        let device = Device::new().with_identity("44010250", "1234567", "Combi");
        let (engine, bus) = connect(device, test_config());

        let id = engine.identity(ADDR).unwrap();
        assert_eq!(id.article, "44010250");
        assert_eq!(id.serial, "1234567");
        assert_eq!(id.name, "Combi");
        assert_eq!(id.firmware, "1.0");

        let after_first = bus.sent_class(can_class::PROPERTY_REQ).len();
        assert_eq!(engine.identity(ADDR).unwrap(), id);
        assert_eq!(bus.sent_class(can_class::PROPERTY_REQ).len(), after_first);
    }

    /// An article number with a trailing space defeats every catalog and
    /// mapping lookup that keys on it, so identity trims what it fetches.
    #[test]
    fn identity_strings_are_trimmed() {
        let device = Device::new().with_identity("44010250 ", "1234567", "Combi");
        let (engine, _bus) = connect(device, test_config());

        assert_eq!(engine.identity(ADDR).unwrap().article, "44010250");
    }

    // ── subscriptions ───────────────────────────────────────────────────────

    #[test]
    fn a_subscription_delivers_values_at_its_interval() {
        let device = Device::new().with_btm1(0x17, 12.5);
        let (engine, _bus) = connect(device, test_config());
        let f = field_id::btm1(0x17);
        seed_schema(&engine, vec![field(f, "Voltage", VisualizationType::Float)]);

        let (_id, rx) = engine.subscribe(ADDR, vec![f], Duration::from_millis(10), false);
        for _ in 0..2 {
            let update = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(update.device, ADDR);
            assert_eq!(update.field, f);
            assert_eq!(update.value, Value::Float(12.5));
        }
    }

    /// `change_only` suppresses repeats of an unchanged value, and delivers
    /// again the moment the device reports something new.
    #[test]
    fn change_only_delivers_only_on_change() {
        let device = Device::new().with_btm1(0x17, 12.5);
        let (engine, bus) = connect(device, test_config());
        let f = field_id::btm1(0x17);
        seed_schema(&engine, vec![field(f, "Voltage", VisualizationType::Float)]);

        let (_id, rx) = engine.subscribe(ADDR, vec![f], Duration::from_millis(10), true);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap().value,
            Value::Float(12.5)
        );
        assert!(rx.recv_timeout(Duration::from_millis(60)).is_err());

        bus.with_device(|d| {
            d.btm1.insert(0x17, 13.0f32.to_le_bytes());
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap().value,
            Value::Float(13.0)
        );
    }

    #[test]
    fn unsubscribing_stops_the_updates() {
        let device = Device::new().with_btm1(0x17, 12.5);
        let (engine, _bus) = connect(device, test_config());
        let f = field_id::btm1(0x17);
        seed_schema(&engine, vec![field(f, "Voltage", VisualizationType::Float)]);

        let (id, rx) = engine.subscribe(ADDR, vec![f], Duration::from_millis(10), false);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        engine.unsubscribe(id);

        // Drain whatever was already in flight, then expect silence.
        while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    // ── bus master ──────────────────────────────────────────────────────────

    /// With `heartbeat_master` set the scheduler drives the bus itself: the
    /// first heartbeat goes out immediately on connect, then at the interval.
    #[test]
    fn heartbeats_are_emitted_when_acting_as_bus_master() {
        let config = Config {
            heartbeat_master: Some(0x53A493),
            heartbeat_interval: Duration::from_millis(10),
            ..test_config()
        };
        let (_engine, bus) = connect(Device::new(), config);

        let deadline = Instant::now() + Duration::from_secs(2);
        while bus.sent_class(can_class::BUS_POLL).len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let beats = bus.sent_class(can_class::BUS_POLL);
        assert!(
            beats.len() >= 3,
            "expected repeated heartbeats, got {beats:?}"
        );
        assert!(beats.iter().all(|f| *f == (0x05_53A493, Vec::new())));
    }

    /// A passive engine transmits nothing until it's asked to.
    #[test]
    fn a_passive_engine_stays_off_the_bus() {
        let (_engine, bus) = connect(Device::new(), test_config());
        std::thread::sleep(Duration::from_millis(50));
        assert!(bus.sent().is_empty());
    }
}
