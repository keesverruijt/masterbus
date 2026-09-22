//! Runtime engine: reader thread, single bus scheduler, shared state.

mod discovery;
// Also built behind the `fake-bus` feature, so `masterbus-signalk --fake-bus`
// can run a canned bus for smoke tests and the Signal K plugin's CI.
#[cfg(any(test, feature = "fake-bus"))]
pub mod fakebus;
mod framelog;
mod reader;
mod scheduler;
mod state;
mod waiter;

pub(crate) use state::State;
pub(crate) use waiter::Waiter;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use crate::error::{Error, Result};
use crate::model::{DeviceId, FieldId};
use crate::transport::Transport;
use crate::value::{Value, WriteValue};

/// Tunable, lib-level behaviour.
#[derive(Debug, Clone)]
pub struct Config {
    /// Default freshness for on-demand reads; older cached values trigger a poll.
    pub max_age: Duration,
    /// A device is "alive" if heard within this window.
    pub liveness: Duration,
    /// Minimum spacing between transmitted frames (bus budget).
    pub min_send_interval: Duration,
    /// Per-attempt timeout for discovery request/response round-trips.
    pub discovery_timeout: Duration,
    /// Discovery attempts before giving up on a query.
    pub discovery_retries: usize,
    /// The least `devices_all` waits after connect before reporting the bus.
    /// Devices announce themselves every second or two, so this is how long
    /// a bus with steady, prompt talkers needs to fill in.
    pub discovery_window: Duration,
    /// How long the bus has to be free of *new* devices before `devices_all`
    /// reports it, once `discovery_window` has elapsed. The quiet devices on
    /// a big bus (interfaces, a display, an idle charger) first speak at or
    /// after the two-second mark; extending the wait while new devices keep
    /// arriving catches them without slowing a bus that filled in early.
    /// `Duration::ZERO` restores the fixed window. (#22)
    pub discovery_settle: Duration,
    /// How long `connect` waits to hear the first device before giving up.
    /// `connect` returns as soon as one broadcast is heard, so a generous value
    /// only helps on a quiet or noisy bus; it never slows a healthy one.
    pub connect_timeout: Duration,
    /// Optional on-disk schema cache directory (memory-only if `None`).
    pub cache_path: Option<PathBuf>,
    /// If set, act as a bus master: periodically emit a class-`0x05` heartbeat
    /// from this address. Devices announce (class `0x04`) and stay responsive in
    /// response, which is what lets us enumerate the bus when no hardware master
    /// is present. Leave `None` to stay passive (a hardware master must drive the
    /// bus, e.g. an EasyView panel).
    pub heartbeat_master: Option<DeviceId>,
    /// Interval between heartbeats when `heartbeat_master` is set.
    pub heartbeat_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_age: Duration::from_millis(1500),
            liveness: Duration::from_secs(5),
            min_send_interval: Duration::from_millis(1),
            discovery_timeout: Duration::from_millis(150),
            discovery_retries: 3,
            discovery_window: Duration::from_millis(2000),
            discovery_settle: Duration::from_millis(2000),
            connect_timeout: Duration::from_secs(15),
            cache_path: None,
            heartbeat_master: None,
            heartbeat_interval: Duration::from_secs(1),
        }
    }
}

/// A value update delivered to a subscriber.
#[derive(Debug, Clone)]
pub struct ValueUpdate {
    /// Device id.
    pub device: DeviceId,
    /// Channel-aware field id.
    pub field: FieldId,
    /// The new value.
    pub value: Value,
}

/// Device presence change.
#[derive(Debug, Clone, Copy)]
pub enum DeviceEvent {
    /// A device became (or is) alive.
    Alive(DeviceId),
    /// A device went offline (stopped broadcasting).
    Offline(DeviceId),
}

/// Internal subscription spec.
pub(crate) struct SubSpec {
    pub id: u64,
    pub device: DeviceId,
    pub fields: Vec<FieldId>,
    pub interval: Duration,
    pub change_only: bool,
    pub sender: Sender<ValueUpdate>,
}

/// Commands sent from the API to the scheduler thread.
pub(crate) enum Command {
    Identify {
        addr: DeviceId,
        reply: Sender<Result<()>>,
    },
    DiscoverMenu {
        addr: DeviceId,
        menu: crate::model::Menu,
        reply: Sender<Result<()>>,
    },
    Discover {
        addr: DeviceId,
        reply: Sender<Result<()>>,
    },
    DiscoverAllFields {
        addr: DeviceId,
        reply: Sender<Result<()>>,
    },
    Read {
        addr: DeviceId,
        field: FieldId,
        max_age: Duration,
        reply: Sender<Result<Value>>,
    },
    Write {
        addr: DeviceId,
        field: FieldId,
        value: WriteValue,
        reply: Sender<Result<Value>>,
    },
    WriteString {
        addr: DeviceId,
        str_id: u16,
        text: String,
        reply: Sender<Result<()>>,
    },
    AccessLevelRead {
        addr: DeviceId,
        reply: Sender<Result<crate::model::AccessLevel>>,
    },
    AccessLevelSet {
        addr: DeviceId,
        level: crate::model::AccessLevel,
        /// `None` ⇒ logout (no code on the wire); `Some(f32)` ⇒ login with
        /// the caller-supplied code.
        code: Option<f32>,
        reply: Sender<Result<crate::model::AccessLevel>>,
    },
    Subscribe(SubSpec),
    Unsubscribe(u64),
    Shutdown,
}

/// The most `devices_all` waits after start, however long new devices keep
/// arriving; keeps a bus with a very slow talker from stalling a caller.
const DEVICE_LIST_MAX: Duration = Duration::from_secs(10);

/// When `devices_all` may report the bus: the later of the fixed window and
/// the settle period after the newest device, capped at [`DEVICE_LIST_MAX`].
/// Pure, so the policy can be tested without a bus.
fn device_list_deadline(
    started: Instant,
    window: Duration,
    settle: Duration,
    newest_first_seen: Option<Instant>,
) -> Instant {
    let earliest = started + window;
    let settled = newest_first_seen
        .filter(|_| !settle.is_zero())
        .map(|t| t + settle)
        .unwrap_or(earliest);
    earliest.max(settled).min(started + DEVICE_LIST_MAX)
}

/// The shared engine behind every API handle.
pub(crate) struct Engine {
    pub state: Arc<State>,
    pub config: Config,
    started: Instant,
    cmd_tx: Sender<Command>,
    device_events: Receiver<DeviceEvent>,
    next_sub_id: AtomicU64,
    shutdown: Arc<AtomicBool>,
    _reader: JoinHandle<()>,
    _scheduler: JoinHandle<()>,
}

impl Engine {
    /// Connect over a transport; returns once the bus is usable and ≥1 device is
    /// heard (or `connect_timeout` elapses).
    pub fn connect(transport: Box<dyn Transport>, config: Config) -> Result<Arc<Engine>> {
        let started = Instant::now();
        let (rx, tx) = transport.split();
        let state = Arc::new(State::new());
        let waiter = Arc::new(Waiter::new());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = unbounded::<Command>();
        let (dev_tx, dev_rx) = unbounded::<DeviceEvent>();

        let reader = reader::spawn(
            rx,
            state.clone(),
            waiter.clone(),
            dev_tx,
            shutdown.clone(),
            config.clone(),
        );
        let scheduler = scheduler::spawn(
            tx,
            state.clone(),
            waiter.clone(),
            cmd_rx,
            shutdown.clone(),
            config.clone(),
        );

        // Quick init: wait only until at least one device has been heard.
        let deadline = Instant::now() + config.connect_timeout;
        while !state.any_device() {
            if Instant::now() >= deadline {
                log::error!(
                    "no devices heard on the bus within {:?}",
                    config.connect_timeout
                );
                return Err(Error::Connection("no devices heard on the bus".into()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        log::info!(
            "connected (first device after {:?}, heartbeat_master={:?})",
            started.elapsed(),
            config.heartbeat_master,
        );

        Ok(Arc::new(Engine {
            state,
            config,
            started,
            cmd_tx,
            device_events: dev_rx,
            next_sub_id: AtomicU64::new(1),
            shutdown,
            _reader: reader,
            _scheduler: scheduler,
        }))
    }

    fn call<T>(&self, make: impl FnOnce(Sender<T>) -> Command) -> Result<T> {
        let (tx, rx) = bounded::<T>(1);
        self.cmd_tx
            .send(make(tx))
            .map_err(|_| Error::Connection("engine stopped".into()))?;
        rx.recv()
            .map_err(|_| Error::Connection("engine stopped".into()))
    }

    /// Ensure a device's identity is known (cheap discovery; blocks until ready).
    pub fn ensure_identity(&self, addr: DeviceId) -> Result<()> {
        if self.state.has_identity(addr) {
            return Ok(());
        }
        self.call(|reply| Command::Identify { addr, reply })?
    }

    /// Fetch (cheaply, identity-only) a device's identity.
    pub fn identity(&self, addr: DeviceId) -> Result<crate::model::DeviceIdentity> {
        self.ensure_identity(addr)?;
        self.state.identity(addr).ok_or(Error::NotReady)
    }

    /// Ensure a device's full schema (all menus) is discovered.
    pub fn ensure_schema(&self, addr: DeviceId) -> Result<()> {
        if self.state.has_menus(addr, &discovery::MENUS) {
            return Ok(());
        }
        self.call(|reply| Command::Discover { addr, reply })?
    }

    /// Ensure one menu's groups are discovered (the lazy unit).
    pub fn ensure_menu(&self, addr: DeviceId, menu: crate::model::Menu) -> Result<()> {
        if self.state.has_menu(addr, menu) {
            return Ok(());
        }
        self.call(|reply| Command::DiscoverMenu { addr, menu, reply })?
    }

    /// Ensure a specific field is discovered. For Btm1 fields this falls back
    /// to a full per-menu discovery; for Btm3 fields it falls back to the flat
    /// `all_fields` probe (Btm3 fields don't live in `schema.groups`).
    pub fn ensure_field(&self, addr: DeviceId, field: FieldId) -> Result<()> {
        if self.state.has_field(addr, field) {
            return Ok(());
        }
        match crate::model::field_id::channel(field) {
            crate::model::Channel::Btm1 => self.call(|reply| Command::Discover { addr, reply })?,
            crate::model::Channel::Btm3 => self.ensure_all_fields(addr),
        }
    }

    /// Ensure the flat field enumeration (selector `0x01` probe of every index)
    /// is populated. Used for devices like the Magic-class Nav Chg whose
    /// per-menu group counts (`0x08 0x03`) under-report their schema.
    pub fn ensure_all_fields(&self, addr: DeviceId) -> Result<()> {
        if self.state.has_all_fields(addr) {
            return Ok(());
        }
        self.call(|reply| Command::DiscoverAllFields { addr, reply })?
    }

    /// Read a field value (cache if fresh enough, else poll).
    pub fn read(&self, addr: DeviceId, field: FieldId, max_age: Duration) -> Result<Value> {
        self.call(|reply| Command::Read {
            addr,
            field,
            max_age,
            reply,
        })?
    }

    /// Write a field value; returns the resulting value observed afterwards.
    pub fn write(&self, addr: DeviceId, field: FieldId, value: WriteValue) -> Result<Value> {
        self.call(|reply| Command::Write {
            addr,
            field,
            value,
            reply,
        })?
    }

    /// Write a string to a device's string table at the given id (PROTOCOL.md
    /// §4.4). Used for editable Text-viz fields (Device name etc.); the caller
    /// supplies the string id directly because the metadata path that maps a
    /// Text field to its writable sid isn't yet characterised.
    pub fn write_string(&self, addr: DeviceId, str_id: u16, text: &str) -> Result<()> {
        self.call(|reply| Command::WriteString {
            addr,
            str_id,
            text: text.to_string(),
            reply,
        })?
    }

    /// Read the device's current access level (PROTOCOL.md §4.5).
    pub fn access_level(&self, addr: DeviceId) -> Result<crate::model::AccessLevel> {
        self.call(|reply| Command::AccessLevelRead { addr, reply })?
    }

    /// Send a login request at `level` with the caller-supplied f32 `code`,
    /// or — when `code` is `None` — a logout. Returns the level the device
    /// reports after the request; compare to the prior level to detect a
    /// wrong code (the device silently keeps you at your previous level).
    pub fn set_access_level(
        &self,
        addr: DeviceId,
        level: crate::model::AccessLevel,
        code: Option<f32>,
    ) -> Result<crate::model::AccessLevel> {
        self.call(|reply| Command::AccessLevelSet {
            addr,
            level,
            code,
            reply,
        })?
    }

    /// Subscribe to live updates of `fields` at `interval`. Returns the
    /// subscription id and the update receiver.
    pub fn subscribe(
        &self,
        device: DeviceId,
        fields: Vec<FieldId>,
        interval: Duration,
        change_only: bool,
    ) -> (u64, Receiver<ValueUpdate>) {
        let id = self.next_sub_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = unbounded::<ValueUpdate>();
        let _ = self.cmd_tx.send(Command::Subscribe(SubSpec {
            id,
            device,
            fields,
            interval,
            change_only,
            sender: tx,
        }));
        (id, rx)
    }

    /// Cancel a subscription.
    pub fn unsubscribe(&self, id: u64) {
        let _ = self.cmd_tx.send(Command::Unsubscribe(id));
    }

    /// Currently-alive device ids.
    pub fn device_ids(&self) -> Vec<DeviceId> {
        self.state.alive_ids(self.config.liveness)
    }

    /// Wait until the broadcast-collection window has elapsed since start and
    /// no new device has been heard for the settle period, then return all
    /// alive device ids (the full bus). The deadline moves out each time a
    /// device is first heard, so it is re-evaluated as the wait goes on.
    pub fn device_ids_all(&self) -> Vec<DeviceId> {
        loop {
            let target = device_list_deadline(
                self.started,
                self.config.discovery_window,
                self.config.discovery_settle,
                self.state.newest_first_seen(),
            );
            let now = Instant::now();
            if target <= now {
                break;
            }
            std::thread::sleep((target - now).min(Duration::from_millis(100)));
        }
        self.device_ids()
    }

    /// Device presence event stream.
    pub fn device_events(&self) -> Receiver<DeviceEvent> {
        self.device_events.clone()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.cmd_tx.send(Command::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(2);
    const SETTLE: Duration = Duration::from_secs(2);

    /// A bus that has filled in early reports at the fixed window, as before.
    #[test]
    fn a_quiet_bus_reports_at_the_window() {
        let t0 = Instant::now();
        assert_eq!(device_list_deadline(t0, WINDOW, SETTLE, None), t0 + WINDOW);
        assert_eq!(
            device_list_deadline(t0, WINDOW, SETTLE, Some(t0)),
            t0 + WINDOW
        );
    }

    /// The #22 bus: six devices first speak at the two-second mark. The old
    /// fixed window took its snapshot at exactly that moment and missed them;
    /// now the deadline moves out to two seconds after the newest arrival.
    #[test]
    fn a_late_arrival_extends_the_wait() {
        let t0 = Instant::now();
        let late = t0 + Duration::from_millis(2000);
        assert_eq!(
            device_list_deadline(t0, WINDOW, SETTLE, Some(late)),
            late + SETTLE
        );
    }

    /// A device that keeps arriving cannot stall the caller for ever.
    #[test]
    fn the_wait_is_capped() {
        let t0 = Instant::now();
        let very_late = t0 + Duration::from_secs(30);
        assert_eq!(
            device_list_deadline(t0, WINDOW, SETTLE, Some(very_late)),
            t0 + DEVICE_LIST_MAX
        );
    }

    /// A zero settle period is the old behaviour: the fixed window only.
    #[test]
    fn zero_settle_is_the_fixed_window() {
        let t0 = Instant::now();
        let late = t0 + Duration::from_secs(5);
        assert_eq!(
            device_list_deadline(t0, WINDOW, Duration::ZERO, Some(late)),
            t0 + WINDOW
        );
    }
}
