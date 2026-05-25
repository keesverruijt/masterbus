//! Runtime engine: reader thread, single bus scheduler, shared state.

mod discovery;
mod reader;
mod scheduler;
mod state;
mod waiter;

pub(crate) use state::State;
pub(crate) use waiter::Waiter;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

use crate::error::{Error, Result};
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
    /// How long `connect` waits to hear the first device.
    pub connect_timeout: Duration,
    /// Optional on-disk schema cache directory (memory-only if `None`).
    pub cache_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_age: Duration::from_millis(1500),
            liveness: Duration::from_secs(5),
            min_send_interval: Duration::from_millis(3),
            discovery_timeout: Duration::from_millis(150),
            discovery_retries: 3,
            connect_timeout: Duration::from_secs(3),
            cache_path: None,
        }
    }
}

/// A value update delivered to a subscriber.
#[derive(Debug, Clone)]
pub struct ValueUpdate {
    /// Device id.
    pub device: u32,
    /// Field index.
    pub field: i32,
    /// The new value.
    pub value: Value,
}

/// Device presence change.
#[derive(Debug, Clone, Copy)]
pub enum DeviceEvent {
    /// A device became (or is) alive.
    Alive(u32),
    /// A device went offline (stopped broadcasting).
    Offline(u32),
}

/// Internal subscription spec.
pub(crate) struct SubSpec {
    pub id: u64,
    pub device: u32,
    pub fields: Vec<i32>,
    pub interval: Duration,
    pub change_only: bool,
    pub sender: Sender<ValueUpdate>,
}

/// Commands sent from the API to the scheduler thread.
pub(crate) enum Command {
    Identify { addr: u32, reply: Sender<Result<()>> },
    Discover { addr: u32, reply: Sender<Result<()>> },
    Read { addr: u32, field: i32, max_age: Duration, reply: Sender<Result<Value>> },
    Write { addr: u32, field: i32, value: WriteValue, reply: Sender<Result<Value>> },
    Subscribe(SubSpec),
    Unsubscribe(u64),
    Shutdown,
}

/// How long after start `devices_all` waits for the broadcast list to fill.
const DEVICE_LIST_WINDOW: Duration = Duration::from_millis(2000);

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

        let reader = reader::spawn(rx, state.clone(), waiter.clone(), dev_tx, shutdown.clone(), config.clone());
        let scheduler =
            scheduler::spawn(tx, state.clone(), waiter.clone(), cmd_rx, shutdown.clone(), config.clone());

        // Quick init: wait only until at least one device has been heard.
        let deadline = Instant::now() + config.connect_timeout;
        while !state.any_device() {
            if Instant::now() >= deadline {
                return Err(Error::Connection("no devices heard on the bus".into()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }

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
        self.cmd_tx.send(make(tx)).map_err(|_| Error::Connection("engine stopped".into()))?;
        rx.recv().map_err(|_| Error::Connection("engine stopped".into()))
    }

    /// Ensure a device's identity is known (cheap discovery; blocks until ready).
    pub fn ensure_identity(&self, addr: u32) -> Result<()> {
        if self.state.has_identity(addr) {
            return Ok(());
        }
        self.call(|reply| Command::Identify { addr, reply })?
    }

    /// Fetch (cheaply, identity-only) a device's identity.
    pub fn identity(&self, addr: u32) -> Result<crate::model::DeviceIdentity> {
        self.ensure_identity(addr)?;
        self.state.identity(addr).ok_or(Error::NotReady)
    }

    /// Ensure a device's schema is discovered (blocks until ready or timeout).
    pub fn ensure_schema(&self, addr: u32) -> Result<()> {
        if self.state.has_schema(addr) {
            return Ok(());
        }
        self.call(|reply| Command::Discover { addr, reply })?
    }

    /// Read a field value (cache if fresh enough, else poll).
    pub fn read(&self, addr: u32, field: i32, max_age: Duration) -> Result<Value> {
        self.call(|reply| Command::Read { addr, field, max_age, reply })?
    }

    /// Write a field value; returns the resulting value observed afterwards.
    pub fn write(&self, addr: u32, field: i32, value: WriteValue) -> Result<Value> {
        self.call(|reply| Command::Write { addr, field, value, reply })?
    }

    /// Subscribe to live updates of `fields` at `interval`. Returns the
    /// subscription id and the update receiver.
    pub fn subscribe(
        &self,
        device: u32,
        fields: Vec<i32>,
        interval: Duration,
        change_only: bool,
    ) -> (u64, Receiver<ValueUpdate>) {
        let id = self.next_sub_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = unbounded::<ValueUpdate>();
        let _ = self
            .cmd_tx
            .send(Command::Subscribe(SubSpec { id, device, fields, interval, change_only, sender: tx }));
        (id, rx)
    }

    /// Cancel a subscription.
    pub fn unsubscribe(&self, id: u64) {
        let _ = self.cmd_tx.send(Command::Unsubscribe(id));
    }

    /// Currently-alive device ids.
    pub fn device_ids(&self) -> Vec<u32> {
        self.state.alive_ids(self.config.liveness)
    }

    /// Wait until the broadcast-collection window has elapsed since start, then
    /// return all alive device ids (the full bus).
    pub fn device_ids_all(&self) -> Vec<u32> {
        let target = self.started + DEVICE_LIST_WINDOW;
        let now = Instant::now();
        if target > now {
            std::thread::sleep(target - now);
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
