//! Single bus-TX thread: discovery (high priority), on-demand reads/writes, and
//! rate-based subscription polling — paced to a bus budget, passive-first.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};

use super::discovery::{discover_menu, fetch_identity, Disc, MENUS};
use crate::model::{DeviceIdentity, Menu};
use super::reader::value_key;
use super::state::State;
use super::waiter::Waiter;
use super::{Command, Config, SubSpec, ValueUpdate};
use crate::error::{Error, Result};
use crate::model::AccessLevel;
use crate::protocol::{
    decode_value, encode_commit, encode_login_read, encode_login_write, encode_logout,
    encode_set_boolean, encode_set_float, encode_set_list, heartbeat_raw, monitoring_req_raw,
    TAB_DEFAULT, VisualizationType,
};
use crate::transport::TransportTx;
use crate::value::{Value, WriteValue};

const VALUE_READ_TIMEOUT: Duration = Duration::from_millis(500);

struct SubState {
    spec: SubSpec,
    next_due: HashMap<i32, Instant>,
    last_value: HashMap<i32, Value>,
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
            Sched { tx, state, waiter, config, last_send: Instant::now(), next_heartbeat }
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
                Ok(Command::Read { addr, field, max_age, reply }) => {
                    let r = self.do_read(addr, field, max_age);
                    let _ = reply.send(r);
                }
                Ok(Command::Write { addr, field, value, reply }) => {
                    let r = self.do_write(addr, field, value);
                    let _ = reply.send(r);
                }
                Ok(Command::AccessLevelRead { addr, reply }) => {
                    let r = self.do_access_level_read(addr);
                    let _ = reply.send(r);
                }
                Ok(Command::AccessLevelSet { addr, level, reply }) => {
                    let r = self.do_access_level_set(addr, level);
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
        let Some(master) = self.config.heartbeat_master else { return };
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
        let _ = self.tx.send(frame.0, &frame.1);
    }

    /// Fetch (and cache) just a device's identity — the cheap half of discovery.
    fn do_identify(&mut self, addr: u32) {
        if self.state.has_identity(addr) {
            return;
        }
        let id = self.with_disc(|disc| fetch_identity(disc, addr));
        self.state.put_identity(addr, id);
    }

    /// Ensure the identity is known (reusing the cache), returning it.
    fn ensure_identity(&mut self, addr: u32) -> DeviceIdentity {
        if let Some(id) = self.state.identity(addr) {
            return id;
        }
        let id = self.with_disc(|disc| fetch_identity(disc, addr));
        self.state.put_identity(addr, id.clone());
        id
    }

    /// Discover one menu's groups for a device (the lazy unit of discovery).
    fn do_discover_menu(&mut self, addr: u32, menu: Menu) {
        if self.state.has_menu(addr, menu) {
            return;
        }
        let id = self.ensure_identity(addr);
        // Discover this device's own menu (per-device disk cache inside). We do
        // not reuse another same-article device's schema: devices that share an
        // article can differ (e.g. the cluster master battery has an extra group).
        let groups = self.with_disc(|disc| discover_menu(disc, addr, &id, menu));
        self.last_send = Instant::now();
        self.state.put_menu(addr, menu, groups);
    }

    /// Discover every menu (a full schema).
    fn do_discover_all(&mut self, addr: u32) {
        for menu in MENUS {
            self.do_discover_menu(addr, menu);
        }
    }

    /// Run a closure with a `Disc` bound to this scheduler's transport.
    fn with_disc<T>(&mut self, f: impl FnOnce(&mut Disc) -> T) -> T {
        let waiter = self.waiter.clone();
        let cfg = self.config.clone();
        let mut disc = Disc { tx: &mut *self.tx, waiter: &waiter, cfg: &cfg };
        f(&mut disc)
    }

    fn viz_of(&mut self, addr: u32, field: i32) -> Result<VisualizationType> {
        // If the field isn't known yet, fall back to a full discovery (we don't
        // know which menu it belongs to).
        if !self.state.has_field(addr, field) {
            self.do_discover_all(addr);
        }
        self.state
            .schema(addr)
            .and_then(|s| s.field(field).map(|f| f.viz_type))
            .ok_or(Error::FieldNotAvailable(field))
    }

    fn poll_value(&mut self, addr: u32, field: i32, viz: VisualizationType) -> Result<Value> {
        let key = value_key(addr, field);
        self.waiter.register(&key);
        self.send(monitoring_req_raw(addr, field as u8, TAB_DEFAULT));
        match self.waiter.wait(&key, VALUE_READ_TIMEOUT) {
            Some(raw) => {
                let opts = self
                    .state
                    .schema(addr)
                    .and_then(|s| s.field(field).map(|f| f.options.clone()))
                    .unwrap_or_default();
                let v = decode_value(&raw, viz).with_options(&opts);
                self.state.put_value(addr, field, v.clone());
                Ok(v)
            }
            None => Err(Error::Timeout),
        }
    }

    fn do_read(&mut self, addr: u32, field: i32, max_age: Duration) -> Result<Value> {
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

    fn await_access_level(&mut self, addr: u32) -> Result<AccessLevel> {
        let key = format!("p:{:06X}:08:19", addr);
        match self.waiter.wait(&key, VALUE_READ_TIMEOUT) {
            Some(data) if data.len() >= 3 => {
                AccessLevel::from_byte(data[2]).ok_or_else(|| {
                    Error::Protocol(format!("unknown access level byte 0x{:02X}", data[2]))
                })
            }
            Some(_) => Err(Error::Protocol("short access-level response".into())),
            None => Err(Error::Timeout),
        }
    }

    fn do_access_level_read(&mut self, addr: u32) -> Result<AccessLevel> {
        let key = format!("p:{:06X}:08:19", addr);
        self.waiter.register(&key);
        self.send(encode_login_read(addr));
        self.await_access_level(addr)
    }

    fn do_access_level_set(&mut self, addr: u32, level: AccessLevel) -> Result<AccessLevel> {
        let key = format!("p:{:06X}:08:19", addr);
        self.waiter.register(&key);
        let frame = match level.code() {
            Some(code) => encode_login_write(addr, level.level_byte(), code),
            None => encode_logout(addr),
        };
        self.send(frame);
        let reported = self.await_access_level(addr)?;
        // A level change flips per-field WRITEABLE (shadow op 0x0B) on many
        // fields; the schema we cached at the prior level is now stale. Drop
        // it so the next field access re-runs discovery with fresh shadow
        // attributes. (PROTOCOL.md §4.5.)
        self.state.forget_schema(addr);
        Ok(reported)
    }

    fn do_write(&mut self, addr: u32, field: i32, value: WriteValue) -> Result<Value> {
        let frame = match value {
            WriteValue::Bool(b) => encode_set_boolean(addr, field as u8, b),
            WriteValue::Float(f) => encode_set_float(addr, field as u8, f),
            WriteValue::ListIndex(i) => encode_set_list(addr, field as u8, i),
        };
        self.send(frame);
        // Relay-style boolean controls (e.g. the CombiMaster inverter/charger)
        // only act when the value write is followed by a fixed "commit" token to
        // the adjacent hidden command register at field+1. Only emit it when that
        // register is hidden (not a real schema field), so we never clobber a
        // neighbouring setting on devices that don't use this pattern.
        if matches!(value, WriteValue::Bool(_)) {
            let cmd = field + 1;
            let cmd_hidden = self
                .state
                .schema(addr)
                .map(|s| s.field(cmd).is_none())
                .unwrap_or(true);
            if cmd_hidden {
                self.send(encode_commit(addr, cmd as u8));
            }
        }
        self.state.mark_outdated(addr, field);
        // Confirm by observing the resulting value.
        let viz = self.viz_of(addr, field)?;
        self.poll_value(addr, field, viz)
    }

    // ── subscriptions ───────────────────────────────────────────────────────

    fn add_sub(&mut self, subs: &mut Vec<SubState>, spec: SubSpec) {
        // Ensure the subscribed fields' types are known. We don't know which menus
        // they live in, so discover the full schema (one-time; disk-cached).
        if !spec.fields.iter().all(|&f| self.state.has_field(spec.device, f)) {
            self.do_discover_all(spec.device);
        }
        let now = Instant::now();
        let next_due = spec.fields.iter().map(|&f| (f, now)).collect();
        subs.push(SubState { spec, next_due, last_value: HashMap::new() });
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
        let mut work: Vec<(usize, i32)> = Vec::new();
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
                    let _ = s.spec.sender.send(ValueUpdate { device, field, value: v });
                }
            }
        }
    }
}
