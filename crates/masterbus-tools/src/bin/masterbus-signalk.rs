//! Signal K daemon for Mastervolt MasterBus.
//!
//! Subscribes to the fields a curated mapping names and serves **Signal K
//! deltas** as newline-delimited JSON over **TCP** (`0.0.0.0:3009` by
//! default), and an **HTTP control API** (off by default; `127.0.0.1:3010`
//! when the Signal K plugin runs it) through which the plugin lists devices,
//! edits the mapping and writes fields. The API is documented in
//! `docs/API.md`.
//!
//! ```text
//! masterbus-signalk [listen-addr] [--stream ADDR] [--api ADDR]
//!                   [--api-token-file PATH] [--config-dir DIR]
//! ```
//!
//! Transport (USB / SocketCAN), master role, the schema cache directory, the
//! stream and API addresses and the API token all come from the per-host
//! config file (see `masterbus::FileConfig`); the file is created on first
//! run. Command-line flags override it, and `--config-dir` (or the
//! `MASTERBUS_CONFIG_DIR` environment variable) says where the file is, so a
//! supervisor can keep everything this daemon owns in one directory. Once
//! both listeners are bound the daemon prints one `READY {...}` line on
//! stdout naming them, which is what a supervisor waits for.
//!
//! The daemon shares its bus connection with the masterbus tools on the same
//! machine (`masterbus-tui`, `masterbus-dump`, `masterbus-set-field`): it
//! relays frames on a loopback port it announces in the schema cache folder
//! (see `masterbus::transport::relay`). That is what lets the TUI run while
//! Signal K holds the USB link.
//!
//! # What gets published
//!
//! Exactly what `mapping.json` says, and nothing else. The file sits beside
//! `config.ini`; `MAPPING` overrides the location. It is keyed on device
//! **serial number** and **field id**, because those are what the firmware
//! fixes — device, group and field *names* are installer-editable, and issue
//! #12 has the bus that proves matching on them cannot work.
//!
//! The file is meant to be curated by a human, in the plugin's editor or in
//! `masterbus-tui --mapping`. When it is missing or empty, this daemon seeds
//! one from [`masterbus_tools::seed`] — the bundled per-model database first,
//! then the per-class name heuristics — and writes it out, so an install that
//! worked before keeps working and has something to edit.
//!
//! Unit conversion and unit metadata are **derived** from the field's own
//! unit, never stored (see [`masterbus_tools::units::to_si`]); the target
//! leaf only cross-checks. A mapping entry whose units cannot be reconciled is
//! reported and skipped rather than published as a wrong number. Every such
//! report is a [`masterbus_tools::publish::Diagnostic`]: printed here, and
//! served by the API so the editor can show it next to the entry.
//!
//! The mapping takes effect the moment it is replaced through the API, and
//! the file is also re-read when it changes on disk, so an edit in
//! `masterbus-tui --mapping` lands within a couple of seconds.
//!
//! Each device starts publishing as soon as it is identified, rather than
//! once the whole bus has been walked: identifying a device costs a few dozen
//! round trips, and on a big bus the last one is minutes behind the first
//! (#40). Discovery does not end at startup either. A device that announces
//! itself after the initial pass — a quiet interface, or a charger switched
//! on when shore power is connected — is identified as it appears and, if
//! the mapping names its serial, starts publishing then (#22). A mapping that
//! names a field outside the Monitoring menu (a writable setting exposed as a
//! PUT target) has that device's Configuration menu discovered before it is
//! resolved. All of this runs on one discovery thread, a device at a time,
//! so the stream never waits for it. Each device's identification and the
//! start of its stream are logged with the seconds since start, and served
//! by the API as its `timeline`.
//!
//! An enum whose mapping names alarm labels (`"notify": {"Alarm": "alarm"}`)
//! also drives `notifications.<path>`: the spec's `state` / `method` /
//! `message` object, re-sent on every change and to every new client, so a
//! Signal K server can sound it rather than show a word on a dashboard.
//!
//! Besides live values, each published device also emits static `name` and
//! `manufacturer` metadata once per client connection.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender};
use masterbus::{Config, DeviceEvent, DeviceId, FieldId, MasterBus, Subscription, Value};
use masterbus_tools::api::{self, DeviceTimes, Info, Shared, secs};
use masterbus_tools::mapping::{Mapping, NotifyState};
use masterbus_tools::publish::{self, DeviceRec, Emit};
use masterbus_tools::signalk;
use serde_json::json;

/// Default TCP listen address of the delta stream.
const DEFAULT_LISTEN: &str = "0.0.0.0:3009";

/// How often each value is (re)emitted.
const RATE: Duration = Duration::from_millis(1000);

/// How often the mapping file is checked for a change.
const RELOAD_CHECK: Duration = Duration::from_secs(2);

/// What the command line said.
#[derive(Debug, Default, PartialEq)]
struct Args {
    /// `--stream ADDR`, or the bare positional address of older invocations.
    stream: Option<String>,
    /// `--api ADDR`.
    api: Option<String>,
    /// `--api-token-file PATH`.
    api_token_file: Option<PathBuf>,
    /// `--config-dir DIR`.
    config_dir: Option<PathBuf>,
    /// `--fake-bus`: serve a canned bus instead of real hardware.
    fake_bus: bool,
}

const USAGE: &str = "\
usage: masterbus-signalk [listen-addr] [options]

  --stream ADDR          delta stream listen address (default: `listen` in
                         config.ini, else 0.0.0.0:3009); the bare positional
                         form is the same thing
  --api ADDR             HTTP control API listen address (default: `api_listen`
                         in config.ini, else off). Any address other than
                         loopback needs a token.
  --api-token-file PATH  file holding the API bearer token (default:
                         `api_token` in config.ini)
  --config-dir DIR       where config.ini and mapping.json live (default: the
                         platform path, or $MASTERBUS_CONFIG_DIR)
  --fake-bus             serve a canned three-device bus instead of hardware
                         (builds with the `fake-bus` feature only)
  --version, --help
";

/// Parse the command line; `Err` carries the message to print before exiting.
fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--help" | "-h" => {
                return Err(format!("{USAGE}\n{}", masterbus_tools::config_help()));
            }
            "--version" | "-V" => {
                return Err(format!("masterbus-signalk {}", env!("CARGO_PKG_VERSION")));
            }
            "--stream" => args.stream = Some(value("--stream")?),
            "--api" => args.api = Some(value("--api")?),
            "--api-token-file" => args.api_token_file = Some(value("--api-token-file")?.into()),
            "--config-dir" => args.config_dir = Some(value("--config-dir")?.into()),
            "--fake-bus" => args.fake_bus = true,
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
            s if args.stream.is_none() => args.stream = Some(s.to_string()),
            s => return Err(format!("unexpected argument {s}\n{USAGE}")),
        }
    }
    Ok(args)
}

/// Whether an address only loopback can reach, which is when the API may run
/// without a token.
fn is_loopback(addr: &str) -> bool {
    match addr.parse::<SocketAddr>() {
        Ok(a) => a.ip().is_loopback(),
        Err(_) => addr.starts_with("localhost:"),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(
                if msg.starts_with("usage") || msg.starts_with("masterbus-signalk ") {
                    0
                } else {
                    2
                },
            );
        }
    };
    if let Some(dir) = &args.config_dir {
        // SAFETY: nothing else is running yet; the engine's threads start in
        // `MasterBus::auto` below, after the variable is set.
        unsafe { std::env::set_var(masterbus::settings::CONFIG_DIR_ENV, dir) };
    }
    let file_config = masterbus::FileConfig::load_or_create().ok();
    // Precedence for every setting: command line, then config.ini, then the
    // built-in default. Having them in the config file is what lets the
    // systemd unit drop its own environment file, so this project keeps
    // exactly one configuration directory per host.
    let listen = args
        .stream
        .or_else(|| file_config.as_ref().and_then(|c| c.listen.clone()))
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let api_listen = args
        .api
        .or_else(|| file_config.as_ref().and_then(|c| c.api_listen.clone()));
    let token = match &args.api_token_file {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
            Ok(_) => {
                eprintln!("masterbus-signalk: {} is empty", p.display());
                std::process::exit(2);
            }
            Err(e) => {
                eprintln!("masterbus-signalk: reading {}: {e}", p.display());
                std::process::exit(2);
            }
        },
        None => file_config.as_ref().and_then(|c| c.api_token.clone()),
    };
    if let Some(a) = &api_listen
        && !is_loopback(a)
        && token.is_none()
    {
        eprintln!(
            "masterbus-signalk: the API would listen on {a}, which is not loopback, with no \
             token; set api_token in config.ini or pass --api-token-file"
        );
        std::process::exit(2);
    }
    // Mapping file: `MAPPING` overrides, otherwise it sits beside config.ini.
    // A fake bus has no hardware to auto-detect, so there may be no
    // config.ini; the mapping then lives in `--config-dir` if given.
    let mapping_path: Option<PathBuf> = std::env::var_os("MAPPING")
        .map(PathBuf::from)
        .or_else(|| file_config.as_ref().map(|c| c.mapping_path()))
        .or_else(|| {
            args.fake_bus
                .then(|| args.config_dir.as_ref().map(|d| d.join("mapping.json")))
                .flatten()
        });
    let transport = match &file_config {
        _ if args.fake_bus => "fake".to_string(),
        Some(c) => match c.device_type {
            masterbus::DeviceType::Can => format!("can:{}", c.device_name),
            masterbus::DeviceType::Usb if c.device_name.is_empty() => "usb".to_string(),
            masterbus::DeviceType::Usb => format!("usb:{}", c.device_name),
        },
        None => String::new(),
    };

    // The fake bus must outlive `run`, which never returns; holding it here
    // does that.
    #[cfg(feature = "fake-bus")]
    let _fake: Option<masterbus::fakebus::FakeBus>;
    let bus = if args.fake_bus {
        #[cfg(feature = "fake-bus")]
        {
            let (fake, transport) = masterbus_tools::fake::canned();
            masterbus_tools::fake::animate(&fake);
            _fake = Some(fake);
            eprintln!("masterbus-signalk: serving the canned fake bus (no hardware)");
            let shared = masterbus::transport::relay::share(transport, "127.0.0.1:0", None)
                .and_then(|(t, relay)| {
                    eprintln!("masterbus-signalk: sharing the bus with local tools on {relay}");
                    MasterBus::with_transport(t, Config::default())
                });
            match shared {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("masterbus-signalk: fake bus failed: {e}");
                    std::process::exit(2);
                }
            }
        }
        #[cfg(not(feature = "fake-bus"))]
        {
            eprintln!(
                "masterbus-signalk: this build has no --fake-bus (needs the fake-bus feature)"
            );
            std::process::exit(2);
        }
    } else {
        #[cfg(feature = "fake-bus")]
        {
            _fake = None;
        }
        // The bus is shared with the masterbus tools on this machine, so the
        // TUI can be used while Signal K runs (the USB link takes only one
        // process).
        match MasterBus::auto_shared(Config::default(), "127.0.0.1:0") {
            Ok((b, relay)) => {
                eprintln!("masterbus-signalk: sharing the bus with local tools on {relay}");
                b
            }
            Err(e) => {
                eprintln!("masterbus-signalk: connect failed: {e}");
                std::process::exit(2);
            }
        }
    };
    let opts = Options {
        listen,
        api_listen,
        token,
        mapping_path,
        transport,
    };
    if let Err(e) = run(bus, opts) {
        eprintln!("masterbus-signalk: {e}");
        std::process::exit(1);
    }
}

/// Everything `run` needs beyond the bus.
struct Options {
    listen: String,
    api_listen: Option<String>,
    token: Option<String>,
    mapping_path: Option<PathBuf>,
    transport: String,
}

/// What the discovery thread tells the main loop.
#[derive(Debug, PartialEq)]
enum Found {
    /// A device's record is in `shared.devices`, with whatever its mapping
    /// needs beyond Monitoring already discovered. `mapped` says whether the
    /// mapping names it, which is when the stream has to be re-resolved.
    Device { id: DeviceId, mapped: bool },
    /// The first pass over the bus is done.
    Walked,
}

/// The order devices are identified in: the ones on the bus at start
/// (`first`), then each one that announces itself later, once. One at a
/// time, on one thread, so each device can start publishing as soon as it
/// is identified. (A thread per late device queued every identity request
/// ahead of every menu request on the one scheduler, so none of them
/// finished until nearly all had; #40.)
fn walk(
    first: Vec<DeviceId>,
    events: Receiver<DeviceEvent>,
    mut identify: impl FnMut(DeviceId, bool),
    walked: impl FnOnce(),
) {
    let mut known: HashSet<DeviceId> = HashSet::new();
    for id in first {
        if known.insert(id) {
            identify(id, false);
        }
    }
    walked();
    for ev in events.iter() {
        if let DeviceEvent::Alive(id) = ev
            && known.insert(id)
        {
            identify(id, true);
        }
    }
}

/// Identify one device and add its record to `shared` (the API shows the
/// devices as they arrive). When the mapping names it, what its mapped
/// fields need beyond Monitoring is discovered here too, on the discovery
/// thread, rather than in the main loop, where it would hold up the stream.
fn identify(bus: &MasterBus, shared: &Shared, id: DeviceId, late: bool, found: &Sender<Found>) {
    let t0 = Instant::now();
    let rec = DeviceRec::discover(&bus.device(id));
    let mapping = shared.mapping.lock().unwrap().clone();
    let mapped = mapping.devices.contains_key(&rec.serial);
    {
        let mut list = shared.devices.lock().unwrap();
        if !list.iter().any(|d| d.id == rec.id) {
            list.push(rec.clone());
            list.sort_by_key(|d| d.id);
        }
    }
    if mapped {
        shared.ensure_menus(&mapping);
    }
    let times = DeviceTimes {
        identified: shared.started.elapsed(),
        took: t0.elapsed(),
        late,
        streaming: None,
    };
    let what = if late {
        "joined the bus late,"
    } else {
        "identified"
    };
    let tail = if rec.serial.is_empty() {
        "; it did not identify itself, so it cannot be mapped"
    } else if !late {
        ""
    } else if mapped {
        "; resolving its mapping"
    } else {
        "; it is not in the mapping — add it in the Signal K plugin or `masterbus-tui --mapping`"
    };
    eprintln!(
        "masterbus-signalk: {} ({}) [{:06X}] {what} {:.1} s after start, in {:.1} s{tail}",
        rec.name,
        rec.serial,
        rec.id,
        secs(times.identified),
        secs(times.took),
    );
    shared.timeline.lock().unwrap().devices.insert(id, times);
    let _ = found.send(Found::Device { id, mapped });
}

/// Start the discovery thread: the first pass over the bus, then the
/// devices that join later, for as long as the daemon runs.
fn spawn_discovery(bus: MasterBus, shared: Arc<Shared>) -> Receiver<Found> {
    let (tx, rx) = crossbeam_channel::unbounded::<Found>();
    std::thread::Builder::new()
        .name("masterbus-discovery".into())
        .spawn(move || {
            let events = bus.device_events();
            let mut first: Vec<DeviceId> = bus.devices_all().iter().map(|d| d.id()).collect();
            first.sort();
            eprintln!(
                "masterbus-signalk: {} device(s) on the bus {:.1} s after start; identifying them",
                first.len(),
                secs(shared.started.elapsed()),
            );
            let done = tx.clone();
            walk(
                first,
                events,
                |id, late| identify(&bus, &shared, id, late, &tx),
                || {
                    let at = shared.started.elapsed();
                    shared.timeline.lock().unwrap().walked = Some(at);
                    eprintln!(
                        "masterbus-signalk: first pass over the bus done {:.1} s after start",
                        secs(at)
                    );
                    let _ = done.send(Found::Walked);
                },
            );
        })
        .expect("spawn the discovery thread");
    rx
}

/// What the main loop knows about discovery, for [`activate`].
#[derive(Default)]
struct Progress {
    /// Devices the discovery thread is done with. One still being prepared
    /// is left out of the stream until it is, so its PUT targets are not
    /// reported missing in the meantime.
    ready: HashSet<DeviceId>,
    /// Whether the first pass over the bus is done. Until then, a mapped
    /// device that is not on the bus may just not have been reached yet.
    walked: bool,
    /// Diagnostics already printed, so re-resolving as each device arrives
    /// does not repeat them. Cleared when the mapping changes.
    printed: HashSet<String>,
    /// A first run whose walk seeded nothing (it found no device yet) seeds
    /// from the first device that arrives. Off once something is seeded or
    /// the mapping is replaced, so a mapping emptied on purpose stays empty.
    may_seed: bool,
}

fn run(bus: MasterBus, opts: Options) -> std::io::Result<()> {
    // TCP server: clients (e.g. a Signal K server) connect and receive the delta
    // stream. The listener thread appends new connections to the shared set.
    let listener = TcpListener::bind(&opts.listen)?;
    let stream_addr = listener.local_addr()?;
    eprintln!("masterbus-signalk: listening on {stream_addr} (Signal K delta, ndjson)");
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    // Connections accepted so far. A new one needs the current notification
    // states, which are only ever sent on change.
    let connections: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    // Static per-device metadata (name / manufacturer), rendered once discovery
    // completes. Replayed to every client the moment it connects so late joiners
    // still learn each device's identity without waiting for a value change.
    let static_batch: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

    // The state the API and this loop share. The mapping is loaded below,
    // once the bus has been walked, because seeding needs the devices.
    let (reload_tx, reload_rx) = crossbeam_channel::unbounded::<()>();
    let shared = Arc::new(Shared {
        bus: Box::new(bus.clone()),
        devices: Mutex::new(Vec::new()),
        mapping: Mutex::new(Mapping::new()),
        diagnostics: Mutex::new(Vec::new()),
        values: Mutex::new(HashMap::new()),
        mapping_path: opts.mapping_path.clone(),
        reload: reload_tx,
        started: Instant::now(),
        info: Info {
            transport: opts.transport.clone(),
            stream: stream_addr.to_string(),
            api: opts.api_listen.clone().unwrap_or_default(),
        },
        token: opts.token.clone(),
        streaming: AtomicUsize::new(0),
        clients: AtomicUsize::new(0),
        asked: Mutex::new(std::collections::HashSet::new()),
        outputs: Mutex::new(HashMap::new()),
        timeline: Mutex::new(api::Timeline::default()),
    });

    {
        let clients = clients.clone();
        let static_batch = static_batch.clone();
        let connections = connections.clone();
        let shared = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                connections.fetch_add(1, Ordering::Relaxed);
                let _ = stream.set_nodelay(true);
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                eprintln!(
                    "masterbus-signalk: client connected: {:?}",
                    stream.peer_addr().ok()
                );
                let sb = static_batch.lock().unwrap().clone();
                if !sb.is_empty() {
                    let _ = (&stream).write_all(&sb).and_then(|()| (&stream).flush());
                }
                let mut cs = clients.lock().unwrap();
                cs.push(stream);
                shared.clients.store(cs.len(), Ordering::Relaxed);
            }
        });
    }

    // The control API, when asked for. Bound before READY so the supervisor
    // can connect to both at once.
    let mut api_addr = None;
    if let Some(addr) = &opts.api_listen {
        let server = tiny_http::Server::http(addr)
            .map_err(|e| std::io::Error::other(format!("API listen on {addr}: {e}")))?;
        let bound = server.server_addr().to_string();
        eprintln!(
            "masterbus-signalk: API on http://{bound}/api/ ({})",
            if opts.token.is_some() {
                "bearer token required"
            } else {
                "loopback, no token"
            }
        );
        let server = Arc::new(server);
        let shared = shared.clone();
        std::thread::spawn(move || api::serve(server, shared));
        api_addr = Some(bound);
    }
    // One line a supervisor can wait for.
    println!(
        "READY {}",
        json!({ "stream": stream_addr.to_string(), "api": api_addr, "version": env!("CARGO_PKG_VERSION") })
    );
    let _ = std::io::stdout().flush();

    // Load the curated mapping before the bus is walked, so each device can
    // start publishing as soon as it is identified (#40). A first run has no
    // mapping; one is seeded once the walk is done, because seeding needs
    // the devices.
    let mapping = match &opts.mapping_path {
        Some(p) => Mapping::load(p).unwrap_or_else(|e| {
            eprintln!("masterbus-signalk: {e}; starting from an empty mapping");
            Mapping::new()
        }),
        None => Mapping::new(),
    };
    *shared.mapping.lock().unwrap() = mapping;

    let found = spawn_discovery(bus.clone(), shared.clone());
    let mut progress = Progress::default();
    let mut active = Active::new();
    let mapping_path = opts.mapping_path.as_deref();
    let mut stamp = mtime(mapping_path);
    let mut last_check = Instant::now();

    loop {
        // Devices the discovery thread is done with. A burst of them (a warm
        // cache on a quiet bus) is resolved once.
        let mut changed = false;
        let mut seeded = false;
        while let Ok(f) = found.try_recv() {
            match f {
                Found::Device { id, mapped } => {
                    progress.ready.insert(id);
                    changed |= mapped;
                    // A first run whose walk found nobody (the bus still
                    // starting) seeded nothing: seed from the first device
                    // that does arrive, or it never publishes.
                    if progress.may_seed && seed_if_empty(&shared, mapping_path, false) {
                        progress.may_seed = false;
                        seeded = true;
                        changed = true;
                        stamp = mtime(mapping_path);
                    }
                }
                Found::Walked => {
                    progress.walked = true;
                    changed = true;
                    let first_run = shared.mapping.lock().unwrap().is_empty();
                    seeded = seed_if_empty(&shared, mapping_path, true);
                    progress.may_seed = first_run && !seeded;
                    if seeded {
                        stamp = mtime(mapping_path);
                    }
                }
            }
        }
        if changed {
            active = activate(
                &bus,
                &shared,
                &clients,
                &static_batch,
                active,
                &mut progress,
                seeded,
            );
        }

        // A mapping replaced through the API is already in `shared`; just
        // re-activate. Coalesce a burst of edits into one activation.
        if reload_rx.try_recv().is_ok() {
            while reload_rx.try_recv().is_ok() {}
            eprintln!("masterbus-signalk: mapping replaced over the API; reloading");
            stamp = mtime(mapping_path);
            progress.printed.clear();
            progress.may_seed = false;
            active = activate(
                &bus,
                &shared,
                &clients,
                &static_batch,
                active,
                &mut progress,
                true,
            );
        }

        // Pick up edits made in `masterbus-tui --mapping` without a restart:
        // both field reports on #12 lost time to a sidecar quietly serving
        // the old file.
        if let Some(p) = mapping_path
            && last_check.elapsed() >= RELOAD_CHECK
        {
            last_check = Instant::now();
            let now = mtime(Some(p));
            if now != stamp {
                stamp = now;
                match Mapping::load(p) {
                    Ok(m) => {
                        eprintln!("masterbus-signalk: {} changed; reloading", p.display());
                        *shared.mapping.lock().unwrap() = m;
                        progress.printed.clear();
                        progress.may_seed = false;
                        active = activate(
                            &bus,
                            &shared,
                            &clients,
                            &static_batch,
                            active,
                            &mut progress,
                            true,
                        );
                    }
                    Err(e) => eprintln!("masterbus-signalk: {e}; keeping the previous mapping"),
                }
            }
        }

        // Skip building deltas when nobody is listening (the channels are still
        // drained below so they don't grow unbounded, and the API's value
        // cache is still fed).
        let have_clients = !clients.lock().unwrap().is_empty();
        // A new client has not seen the notification states; forget what was
        // sent so each is re-sent with the next value.
        let conns = connections.load(Ordering::Relaxed);
        if conns != active.connections_seen {
            active.connections_seen = conns;
            active.notified.clear();
        }
        let mut batch: Vec<u8> = Vec::new();
        let mut new_meta: Vec<serde_json::Value> = Vec::new();
        for (_, sub) in active.subs.values() {
            // Coalesce to the latest value per path this cycle: a field can be
            // updated many times between polls (the boat's real masters poll some
            // values rapidly, and we emit those too).
            let mut latest: HashMap<String, serde_json::Value> = HashMap::new();
            while let Some(u) = sub.try_recv() {
                let Some(e) = active.emit.get(&(u.device, u.field)) else {
                    continue;
                };
                shared
                    .values
                    .lock()
                    .unwrap()
                    .insert((u.device, u.field), u.value.clone());
                if !have_clients {
                    continue;
                }
                if let Some(value) =
                    signalk::encode(&u.value, e.plan.conv, e.plan.invert, &e.plan.truth)
                {
                    latest.insert(e.path.clone(), value);
                }
                if let Some((path, value)) = notification(e, &u.value, &mut active.notified) {
                    latest.insert(path, value);
                }
            }
            if !latest.is_empty() {
                // First sighting of a path → publish its unit metadata once.
                for path in latest.keys() {
                    if !active.meta_sent.contains(path)
                        && let Some(units) = active.units.get(path)
                    {
                        new_meta.push(json!({ "path": path, "value": { "units": units } }));
                        active.meta_sent.insert(path.clone());
                    }
                }
                let values: Vec<_> = latest
                    .into_iter()
                    .map(|(path, value)| json!({ "path": path, "value": value }))
                    .collect();
                let delta = json!({
                    "updates": [{
                        "$source": "masterbus",
                        "timestamp": now_rfc3339(),
                        "values": values,
                    }]
                });
                batch.extend_from_slice(serde_json::to_string(&delta).unwrap().as_bytes());
                batch.push(b'\n');
            }
        }
        // Prepend any new unit metadata (so units land before/with the values)
        // and remember it for clients that connect later.
        if !new_meta.is_empty() {
            let delta = json!({
                "updates": [{ "$source": "masterbus", "timestamp": now_rfc3339(), "meta": new_meta }]
            });
            let mut line = serde_json::to_string(&delta).unwrap().into_bytes();
            line.push(b'\n');
            static_batch.lock().unwrap().extend_from_slice(&line);
            line.extend_from_slice(&batch);
            batch = line;
        }
        if !batch.is_empty() {
            let mut cs = clients.lock().unwrap();
            let before = cs.len();
            cs.retain_mut(|c| c.write_all(&batch).and_then(|()| c.flush()).is_ok());
            let dropped = before - cs.len();
            if dropped > 0 {
                eprintln!("masterbus-signalk: {dropped} client(s) disconnected");
                shared.clients.store(cs.len(), Ordering::Relaxed);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The live state a mapping resolves to: what to publish, the subscriptions
/// feeding it, and the unit metadata each path carries. Rebuilt when the
/// mapping changes or a mapped device arrives; a device whose subscribed
/// fields are unchanged keeps its subscription, and dropping the rest
/// unsubscribes them.
struct Active {
    emit: HashMap<(DeviceId, FieldId), Emit>,
    /// Per device, the fields subscribed (sorted) and the subscription.
    subs: HashMap<DeviceId, (Vec<FieldId>, Subscription)>,
    /// Path → unit metadata, from the device units behind each path.
    units: HashMap<String, &'static str>,
    /// Paths whose unit `meta` has already been published this activation.
    meta_sent: HashSet<String>,
    /// Notification path → the state last sent, so only changes go out.
    notified: HashMap<String, Option<NotifyState>>,
    /// The accept counter as of the last poll, to spot new clients.
    connections_seen: usize,
}

impl Active {
    /// Nothing resolved yet.
    fn new() -> Active {
        Active {
            emit: HashMap::new(),
            subs: HashMap::new(),
            units: HashMap::new(),
            meta_sent: HashSet::new(),
            notified: HashMap::new(),
            connections_seen: usize::MAX,
        }
    }
}

/// On a first run there is no mapping: seed one from the devices found and
/// write it out, so there is something to publish and, more importantly,
/// something to edit. Returns whether it seeded one. `report_empty` says
/// whether seeding nothing is worth a log line (at the end of the walk,
/// not again for every device after it).
fn seed_if_empty(shared: &Shared, path: Option<&Path>, report_empty: bool) -> bool {
    if !shared.mapping.lock().unwrap().is_empty() {
        return false;
    }
    let mapping = publish::seed_mapping(&shared.devices.lock().unwrap());
    if mapping.is_empty() && !report_empty {
        return false;
    }
    match path {
        Some(p) if !mapping.is_empty() => match mapping.save(p) {
            Ok(()) => eprintln!(
                "masterbus-signalk: no mapping yet — seeded {} field(s) from the built-in \
                 heuristics and wrote {}. Review it in the Signal K plugin or with \
                 `masterbus-tui --mapping`.",
                mapping.len(),
                p.display()
            ),
            Err(e) => eprintln!("masterbus-signalk: could not write {}: {e}", p.display()),
        },
        _ => eprintln!(
            "masterbus-signalk: no mapping file; using {} heuristic field(s) for this run",
            mapping.len()
        ),
    }
    let seeded = !mapping.is_empty();
    *shared.mapping.lock().unwrap() = mapping;
    seeded
}

/// Which devices keep their subscription and which need a new one. A device
/// whose subscribed fields are unchanged keeps it, so a device joining does
/// not restart every other device's stream (all their fields due at once,
/// on the one scheduler thread the joining device's discovery also needs).
fn plan_subs(
    old: &HashMap<DeviceId, Vec<FieldId>>,
    wanted: &HashMap<DeviceId, Vec<FieldId>>,
) -> (Vec<DeviceId>, Vec<DeviceId>) {
    let mut keep = Vec::new();
    let mut new = Vec::new();
    for (device, fields) in wanted {
        if old.get(device) == Some(fields) {
            keep.push(*device);
        } else {
            new.push(*device);
        }
    }
    keep.sort();
    new.sort();
    (keep, new)
}

/// The diagnostics to print now: each once, and while the first pass over
/// the bus runs, only those about devices already identified. The rest
/// would mostly say "not on the bus" about a device not reached yet.
fn to_print(
    diagnostics: &[publish::Diagnostic],
    ready_serials: &HashSet<&str>,
    progress: &mut Progress,
) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| progress.walked || ready_serials.contains(d.serial.as_str()))
        .map(|d| d.to_string())
        .filter(|line| progress.printed.insert(line.clone()))
        .collect()
}

/// The notification delta a value update calls for, if any: the first
/// sighting of a notifying field, and every change of state after that.
/// `notified` remembers what was last sent per notification path.
fn notification(
    e: &Emit,
    value: &Value,
    notified: &mut HashMap<String, Option<NotifyState>>,
) -> Option<(String, serde_json::Value)> {
    if e.plan.notify.is_empty() {
        return None;
    }
    let label = value.label()?;
    let state = signalk::notify_state(label, &e.plan.notify);
    let path = signalk::notification_path(&e.path);
    if notified.get(&path) == Some(&state) {
        return None;
    }
    notified.insert(path.clone(), state);
    Some((path, signalk::notification_value(&e.device, label, state)))
}

/// Resolve the mapping against the devices discovery is done with, subscribe
/// to what it names, and (re)publish the static per-device metadata.
/// `ensure` first discovers what the mapping needs beyond Monitoring, for a
/// mapping that has just changed; for a device that joins, the discovery
/// thread has done that already, so the stream does not wait for the bus.
fn activate(
    bus: &MasterBus,
    shared: &Shared,
    clients: &Mutex<Vec<TcpStream>>,
    static_batch: &Mutex<Vec<u8>>,
    previous: Active,
    progress: &mut Progress,
    ensure: bool,
) -> Active {
    let mapping = shared.mapping.lock().unwrap().clone();
    if ensure {
        shared.ensure_menus(&mapping);
    }
    let devices: Vec<DeviceRec> = shared
        .devices
        .lock()
        .unwrap()
        .iter()
        .filter(|d| progress.ready.contains(&d.id))
        .cloned()
        .collect();
    let resolved = publish::resolve(&devices, &mapping);
    let ready_serials: HashSet<&str> = devices.iter().map(|d| d.serial.as_str()).collect();
    for line in to_print(&resolved.diagnostics, &ready_serials, progress) {
        eprintln!("masterbus-signalk: {line}");
    }
    let emit = resolved.emit;
    let mut per_device: HashMap<DeviceId, Vec<FieldId>> = HashMap::new();
    for (device, field) in emit.keys() {
        per_device.entry(*device).or_default().push(*field);
    }
    for fields in per_device.values_mut() {
        fields.sort();
    }
    let published: HashSet<DeviceId> = per_device.keys().copied().collect();
    let mut old = previous.subs;
    let old_fields: HashMap<DeviceId, Vec<FieldId>> =
        old.iter().map(|(d, (f, _))| (*d, f.clone())).collect();
    let (keep, new) = plan_subs(&old_fields, &per_device);
    let mut subs = HashMap::new();
    for device in keep {
        if let Some(s) = old.remove(&device) {
            subs.insert(device, s);
        }
    }
    for device in new {
        let fields = per_device.remove(&device).unwrap_or_default();
        let sub = bus.subscribe(device, fields.clone(), RATE, false);
        subs.insert(device, (fields, sub));
    }
    // What is left is no longer wanted, or wanted with other fields.
    drop(old);
    let units: HashMap<String, &'static str> = emit
        .values()
        .filter_map(|e| e.plan.unit.map(|u| (e.path.clone(), u)))
        .collect();

    // Render the static metadata batch and hand it to the accept thread (for
    // future clients) and to any client already connected.
    let sb = static_meta_batch(&devices, &emit, &published);
    *static_batch.lock().unwrap() = sb.clone();
    if !sb.is_empty() {
        let mut cs = clients.lock().unwrap();
        cs.retain_mut(|c| c.write_all(&sb).and_then(|()| c.flush()).is_ok());
    }

    // When each device first streams, for the log and the API (#40).
    let now = shared.started.elapsed();
    {
        let mut timeline = shared.timeline.lock().unwrap();
        for d in devices.iter().filter(|d| published.contains(&d.id)) {
            let times = timeline.devices.entry(d.id).or_default();
            if times.streaming.is_none() {
                times.streaming = Some(now);
                eprintln!(
                    "masterbus-signalk: {} ({}): streaming {} field(s) {:.1} s after start",
                    d.name,
                    d.serial,
                    subs.get(&d.id).map_or(0, |(f, _)| f.len()),
                    secs(now),
                );
            }
        }
    }

    let total: usize = devices.iter().map(|d| d.fields.len()).sum();
    eprintln!(
        "masterbus-signalk: streaming {} of {total} known fields from {} of {} device(s)",
        emit.len(),
        published.len(),
        devices.len(),
    );
    *shared.diagnostics.lock().unwrap() = resolved.diagnostics;
    shared.streaming.store(emit.len(), Ordering::Relaxed);
    Active {
        emit,
        subs,
        units,
        meta_sent: HashSet::new(),
        notified: HashMap::new(),
        connections_seen: usize::MAX, // forces a first send once anyone connects
    }
}

/// The mapping file's modification time, or `None` when it does not exist.
fn mtime(path: Option<&Path>) -> Option<SystemTime> {
    std::fs::metadata(path?).and_then(|m| m.modified()).ok()
}

/// The Signal K nodes a device publishes into, derived from the paths its
/// mapping actually uses.
///
/// This replaces the old per-class table: with arbitrary curated paths there is
/// no class to look up, and a device that publishes into two categories (a
/// CombiMaster is both an inverter and a charger) names both by itself.
fn nodes_of(device: DeviceId, emit: &HashMap<(DeviceId, FieldId), Emit>) -> Vec<String> {
    let mut nodes: Vec<String> = emit
        .iter()
        .filter(|((d, _), _)| *d == device)
        .filter_map(|(_, e)| signalk::node_of(&e.path))
        .collect();
    nodes.sort();
    nodes.dedup();
    nodes
}

/// Build the one-shot Signal K metadata batch: for every published device, its
/// `name` and `manufacturer` (name + model) on each node it publishes into.
/// Values are static, so this is emitted once per client rather than on the
/// poll loop.
fn static_meta_batch(
    devices: &[DeviceRec],
    emit: &HashMap<(DeviceId, FieldId), Emit>,
    published: &HashSet<DeviceId>,
) -> Vec<u8> {
    let mut batch = Vec::new();
    for d in devices {
        if !published.contains(&d.id) {
            continue;
        }
        let mut values = Vec::new();
        for base in nodes_of(d.id, emit) {
            if !d.name.is_empty() {
                values.push(json!({ "path": format!("{base}.name"), "value": d.name }));
            }
            values.push(
                json!({ "path": format!("{base}.manufacturer.name"), "value": "Mastervolt" }),
            );
            if !d.article.is_empty() {
                values.push(
                    json!({ "path": format!("{base}.manufacturer.model"), "value": d.article }),
                );
            }
        }
        if values.is_empty() {
            continue;
        }
        let delta = json!({
            "updates": [{ "$source": "masterbus", "timestamp": now_rfc3339(), "values": values }]
        });
        batch.extend_from_slice(serde_json::to_string(&delta).unwrap().as_bytes());
        batch.push(b'\n');
    }
    batch
}

/// Current UTC time as an ISO-8601 / RFC-3339 string (no date dependency).
fn now_rfc3339() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let (y, mo, day) = civil_from_days((secs / 86400) as i64);
    format!("{y:04}-{mo:02}-{day:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

/// Days since the Unix epoch → (year, month, day). Howard Hinnant's
/// `civil_from_days`, proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use masterbus_tools::mapping::{DeviceMapping, FieldMapping, field_key};
    use masterbus_tools::publish::FieldRec;
    use masterbus_tools::seed;

    fn dev(serial: &str, name: &str, fields: &[(FieldId, &str, &str)]) -> DeviceRec {
        DeviceRec {
            id: 0x100000 + serial.len() as u32,
            serial: serial.into(),
            article: "66026000".into(),
            name: name.into(),
            firmware: "2.14".into(),
            instance: seed::instance_of(name, 0x100000),
            fields: fields
                .iter()
                .map(|(i, n, u)| FieldRec {
                    id: *i,
                    name: n.to_string(),
                    unit: u.to_string(),
                    options: Vec::new(),
                    writable: false,
                    menu: publish::MENU,
                    group: "Battery".into(),
                })
                .collect(),
            menus: vec![publish::MENU],
        }
    }

    fn mli() -> DeviceRec {
        dev(
            "MLI-1",
            "BAT 24V Service",
            &[
                (0x000, "State of charge", "%"),
                (0x001, "Voltage", "V"),
                (0x002, "Current", "A"),
                (0x005, "Temperature", "\u{b0}C"),
                (0x022, "Relay close", ""),
            ],
        )
    }

    fn emit_for(devices: &[DeviceRec], m: &Mapping) -> HashMap<(DeviceId, FieldId), Emit> {
        publish::resolve(devices, m).emit
    }

    #[test]
    fn the_command_line_is_parsed_with_the_old_positional_form_kept() {
        let p = |v: &[&str]| parse_args(v.iter().map(|s| s.to_string()));
        assert_eq!(p(&[]).unwrap(), Args::default());
        assert_eq!(
            p(&["0.0.0.0:3009"]).unwrap().stream.as_deref(),
            Some("0.0.0.0:3009")
        );
        let a = p(&[
            "--stream",
            "127.0.0.1:3009",
            "--api",
            "127.0.0.1:3010",
            "--api-token-file",
            "/run/t",
            "--config-dir",
            "/var/lib/sk",
        ])
        .unwrap();
        assert_eq!(a.stream.as_deref(), Some("127.0.0.1:3009"));
        assert_eq!(a.api.as_deref(), Some("127.0.0.1:3010"));
        assert_eq!(a.api_token_file, Some(PathBuf::from("/run/t")));
        assert_eq!(a.config_dir, Some(PathBuf::from("/var/lib/sk")));
        assert!(p(&["--api"]).unwrap_err().contains("needs a value"));
        assert!(p(&["--bogus"]).unwrap_err().contains("unknown option"));
        assert!(p(&["a", "b"]).unwrap_err().contains("unexpected"));
        assert!(p(&["--help"]).unwrap_err().starts_with("usage"));
        assert!(
            p(&["--version"])
                .unwrap_err()
                .starts_with("masterbus-signalk ")
        );
    }

    #[test]
    fn only_loopback_may_run_the_api_without_a_token() {
        assert!(is_loopback("127.0.0.1:3010"));
        assert!(is_loopback("[::1]:3010"));
        assert!(is_loopback("localhost:3010"));
        assert!(!is_loopback("0.0.0.0:3010"));
        assert!(!is_loopback("192.168.1.5:3010"));
        assert!(!is_loopback("pi.local:3010"));
    }

    /// The one-shot metadata batch: one delta per published device, naming
    /// it and its manufacturer on every node it publishes into.
    #[test]
    fn the_static_metadata_batch_names_each_published_device() {
        let devices = vec![mli()];
        let emit = emit_for(&devices, &publish::seed_mapping(&devices));
        let published: HashSet<DeviceId> = [devices[0].id].into_iter().collect();

        let batch = static_meta_batch(&devices, &emit, &published);
        let text = String::from_utf8(batch).unwrap();

        // Newline-delimited JSON: one delta, newline-terminated.
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with('\n'));
        let delta: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        let values = delta["updates"][0]["values"].as_array().unwrap();

        let paths: Vec<&str> = values.iter().map(|v| v["path"].as_str().unwrap()).collect();
        assert!(paths.contains(&"electrical.batteries.24vService.name"));
        assert!(paths.contains(&"electrical.batteries.24vService.manufacturer.name"));
        assert!(paths.contains(&"electrical.batteries.24vService.manufacturer.model"));
        let maker = values
            .iter()
            .find(|v| v["path"] == "electrical.batteries.24vService.manufacturer.name")
            .unwrap();
        assert_eq!(maker["value"], "Mastervolt");
        assert_eq!(delta["updates"][0]["$source"], "masterbus");
    }

    /// A device that publishes nothing contributes no metadata — an empty
    /// delta would just be noise on every client connection.
    #[test]
    fn an_unpublished_device_contributes_no_metadata() {
        let devices = vec![mli()];
        let emit = emit_for(&devices, &publish::seed_mapping(&devices));
        let batch = static_meta_batch(&devices, &emit, &HashSet::new());
        assert!(batch.is_empty());
    }

    /// A device with neither a name nor an article still gets its
    /// manufacturer published, since that much is always true.
    #[test]
    fn a_nameless_device_still_gets_a_manufacturer() {
        let mut d = mli();
        d.name = String::new();
        d.article = String::new();
        let devices = vec![d];
        // Seed from the named version so the paths exist, then blank it.
        let named = vec![mli()];
        let emit = emit_for(&named, &publish::seed_mapping(&named));
        let published: HashSet<DeviceId> = [devices[0].id].into_iter().collect();

        let text = String::from_utf8(static_meta_batch(&devices, &emit, &published)).unwrap();
        let delta: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        let paths: Vec<&str> = delta["updates"][0]["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec!["electrical.batteries.24vService.manufacturer.name"]
        );
    }

    /// Timestamps are RFC-3339 in UTC with milliseconds — the shape a Signal
    /// K server expects on every delta.
    #[test]
    fn timestamps_are_rfc3339_utc_with_milliseconds() {
        let t = now_rfc3339();
        assert_eq!(t.len(), 24, "{t}");
        assert!(t.ends_with('Z'), "{t}");
        let (date, time) = t.trim_end_matches('Z').split_once('T').unwrap();
        let parts: Vec<&str> = date.split('-').collect();
        assert_eq!(parts.len(), 3);
        let year: i64 = parts[0].parse().unwrap();
        assert!((2020..2100).contains(&year), "{t}");
        let (hms, millis) = time.split_once('.').unwrap();
        assert_eq!(millis.len(), 3);
        let hms: Vec<u32> = hms.split(':').map(|p| p.parse().unwrap()).collect();
        assert!(hms[0] < 24 && hms[1] < 60 && hms[2] < 60, "{t}");
    }

    /// The mapping file is watched by modification time; a path that isn't
    /// there yet simply has none.
    #[test]
    fn the_mapping_file_is_watched_by_modification_time() {
        assert!(mtime(None).is_none());
        let missing = std::env::temp_dir().join("masterbus-no-such-mapping.json");
        let _ = std::fs::remove_file(&missing);
        assert!(mtime(Some(&missing)).is_none());

        let path = std::env::temp_dir().join(format!(
            "masterbus-mapping-test-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, b"{}").unwrap();
        assert!(mtime(Some(&path)).is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nodes_come_from_the_paths_a_device_actually_uses() {
        let devices = vec![mli()];
        let emit = emit_for(&devices, &publish::seed_mapping(&devices));
        assert_eq!(
            nodes_of(devices[0].id, &emit),
            vec!["electrical.batteries.24vService".to_string()]
        );
    }

    #[test]
    fn a_device_spanning_two_categories_names_both() {
        let d = dev(
            "CMR-1",
            "CMR Combi",
            &[
                (0x001, "Battery voltage", "V"),
                (0x002, "Input voltage", "V"),
            ],
        );
        let devices = vec![d];
        let emit = emit_for(&devices, &publish::seed_mapping(&devices));
        assert_eq!(
            nodes_of(devices[0].id, &emit),
            vec![
                "electrical.chargers.combi".to_string(),
                "electrical.inverters.combi".to_string(),
            ]
        );
    }

    #[test]
    fn every_published_leaf_carries_unit_metadata() {
        let devices = vec![mli()];
        let emit = emit_for(&devices, &publish::seed_mapping(&devices));
        for e in emit.values() {
            // Either the leaf has a unit, or it is a string/boolean leaf.
            let unitless = matches!(
                e.path.rsplit('.').next().unwrap_or(""),
                "chargingMode" | "deviceMode" | "enabled" | "name"
            );
            assert!(
                e.plan.unit.is_some() || unitless,
                "{} has neither unit metadata nor a known unitless leaf",
                e.path
            );
        }
    }

    /// Issue #3 asked for per-installation path overrides. In this design every
    /// path *is* an override, so the thing left to guarantee is that pointing a
    /// field somewhere unusual still works and still converts correctly.
    #[test]
    fn a_custom_path_is_honoured_not_second_guessed() {
        let devices = vec![mli()];
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        // Move the battery out of its canonical category entirely.
        dm.fields.insert(
            field_key(0x005),
            FieldMapping {
                path: "electrical.converters.house.temperature".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        let emit = emit_for(&devices, &m);
        let e = &emit[&(devices[0].id, 0x005)];
        assert_eq!(e.path, "electrical.converters.house.temperature");
        // The conversion still follows from the unit, not from the category.
        assert!((e.plan.conv.apply(20.0) - 293.15).abs() < 1e-9);
        // And the identity metadata follows the device to its new home.
        assert_eq!(
            nodes_of(devices[0].id, &emit),
            vec!["electrical.converters.house".to_string()]
        );
    }

    /// The field reports on #12: spec leaves this build had not tabulated,
    /// and a lifetime energy counter with no spec leaf at all. The unit comes
    /// from the device, so all of them carry metadata now.
    #[test]
    fn a_leaf_nobody_tabulated_still_carries_the_devices_unit() {
        let d = dev(
            "SCM-1",
            "SCM Solar Chg",
            &[
                (0x004, "Panel voltage", "V"),
                (0x009, "Total energy", "kWh"),
            ],
        );
        let devices = vec![d];
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        for (id, leaf) in [(0x004u16, "panelVoltage"), (0x009, "totalEnergy")] {
            dm.fields.insert(
                field_key(id),
                FieldMapping {
                    path: format!("electrical.solar.solar-chg.{leaf}"),
                    ..Default::default()
                },
            );
        }
        m.devices.insert("SCM-1".into(), dm);
        let emit = emit_for(&devices, &m);
        let id = devices[0].id;
        assert_eq!(emit[&(id, 0x004)].plan.unit, Some("V"));
        let e = &emit[&(id, 0x009)];
        assert_eq!(e.plan.unit, Some("J"));
        assert_eq!(e.plan.conv.apply(1.0), 3_600_000.0);
        assert!(e.plan.warning.is_none());
    }

    /// An enum onto a boolean leaf publishes booleans when its labels are the
    /// conventional ones, and is skipped (not mis-published as strings) when
    /// they are not and no truth table was supplied.
    #[test]
    fn enums_on_boolean_leaves_need_a_truth_table() {
        let mut d = dev(
            "MCO-1",
            "INT Contact",
            &[(0x001, "State", ""), (0x002, "State", "")],
        );
        d.fields[0].options = vec!["Standby".into(), "Activated".into()];
        d.fields[1].options = vec!["Standby".into(), "On".into(), "Alarm".into()];
        let devices = vec![d];
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        for id in [0x001u16, 0x002] {
            dm.fields.insert(
                field_key(id),
                FieldMapping {
                    path: format!("electrical.switches.contact-{id}.state"),
                    ..Default::default()
                },
            );
        }
        m.devices.insert("MCO-1".into(), dm);
        let emit = emit_for(&devices, &m);
        let id = devices[0].id;
        assert!(emit[&(id, 0x001)].plan.truth["Activated"]);
        assert!(!emit.contains_key(&(id, 0x002)), "Alarm is ambiguous");
    }

    /// Notifications go out on the first sighting and on every change of
    /// state, not on every value, and clear to `normal` when the label leaves
    /// the table.
    #[test]
    fn notifications_follow_state_changes_only() {
        let e = Emit {
            path: "electrical.inverters.inv.inverterMode".into(),
            plan: signalk::plan(
                "electrical.inverters.inv.inverterMode",
                "",
                &[],
                &FieldMapping {
                    path: "electrical.inverters.inv.inverterMode".into(),
                    notify: [("Alarm".to_string(), NotifyState::Alarm)].into(),
                    ..Default::default()
                },
            )
            .unwrap(),
            device: "INT Inverter 1".into(),
            put: false,
        };
        let options: Vec<String> = ["Standby", "On", "Alarm"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let at = |index: i32| Value::List {
            index,
            options: options.clone(),
        };
        let mut notified = HashMap::new();
        // First sighting: normal, said once.
        let (path, v) = notification(&e, &at(1), &mut notified).expect("first sighting");
        assert_eq!(path, "notifications.electrical.inverters.inv.inverterMode");
        assert_eq!(v["state"], "normal");
        assert!(notification(&e, &at(1), &mut notified).is_none());
        assert!(
            notification(&e, &at(0), &mut notified).is_none(),
            "Standby is normal too"
        );
        // Alarm raised.
        let (_, v) = notification(&e, &at(2), &mut notified).expect("alarm");
        assert_eq!(v["state"], "alarm");
        assert_eq!(v["message"], "INT Inverter 1: Alarm");
        assert!(notification(&e, &at(2), &mut notified).is_none());
        // Cleared.
        let (_, v) = notification(&e, &at(1), &mut notified).expect("clear");
        assert_eq!(v["state"], "normal");
        // A field with no table never notifies.
        let plain = Emit {
            plan: signalk::plan("x.y.mode", "", &[], &FieldMapping::default()).unwrap(),
            ..e
        };
        assert!(notification(&plain, &at(2), &mut HashMap::new()).is_none());
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    /// The devices on the bus at start are identified in order, one at a
    /// time, before any that joins later; each device once, however often it
    /// announces itself or goes away and comes back (#40).
    #[test]
    fn devices_are_identified_one_at_a_time_and_once() {
        let (tx, rx) = crossbeam_channel::unbounded();
        for ev in [
            DeviceEvent::Alive(2), // on the bus at start: already done
            DeviceEvent::Alive(7),
            DeviceEvent::Offline(7),
            DeviceEvent::Alive(7),
            DeviceEvent::Offline(9),
            DeviceEvent::Alive(5),
        ] {
            tx.send(ev).unwrap();
        }
        drop(tx);
        let order = std::cell::RefCell::new(Vec::new());
        walk(
            vec![1, 2, 3],
            rx,
            |id, late| {
                order
                    .borrow_mut()
                    .push(format!("{id}{}", if late { "L" } else { "" }))
            },
            || order.borrow_mut().push("walked".into()),
        );
        assert_eq!(
            order.into_inner(),
            ["1", "2", "3", "walked", "7L", "5L"].map(String::from)
        );
    }

    /// A device whose subscribed fields are unchanged keeps its
    /// subscription when another device joins; only a new device, or one
    /// whose fields changed, is subscribed anew.
    #[test]
    fn a_joining_device_leaves_the_other_subscriptions_alone() {
        let old: HashMap<DeviceId, Vec<FieldId>> =
            HashMap::from([(1, vec![0x01, 0x02]), (2, vec![0x05]), (3, vec![0x07])]);
        let wanted: HashMap<DeviceId, Vec<FieldId>> = HashMap::from([
            (1, vec![0x01, 0x02]), // unchanged
            (2, vec![0x05, 0x06]), // a field added to the mapping
            (4, vec![0x01]),       // joined
        ]);
        let (keep, new) = plan_subs(&old, &wanted);
        assert_eq!(keep, [1]);
        assert_eq!(new, [2, 4]);
    }

    /// While the first pass runs, re-resolving as each device arrives prints
    /// only what is new about devices already identified: not a "not on the
    /// bus" for every mapped device not reached yet, and not the same line
    /// again for every device after it.
    #[test]
    fn diagnostics_are_printed_once_and_not_about_devices_not_reached_yet() {
        let diag = |serial: &str, message: &str| publish::Diagnostic {
            severity: publish::Severity::Info,
            serial: serial.into(),
            device: String::new(),
            field: None,
            path: None,
            message: message.into(),
        };
        let diagnostics = [
            diag("MLI-1", "firmware differs"),
            diag("DSD-1", "not on the bus"),
        ];
        let ready = HashSet::from(["MLI-1"]);
        let mut progress = Progress::default();
        assert_eq!(
            to_print(&diagnostics, &ready, &mut progress),
            ["MLI-1: firmware differs"]
        );
        assert!(to_print(&diagnostics, &ready, &mut progress).is_empty());
        // Once the pass is done, a device still missing is news.
        progress.walked = true;
        assert_eq!(
            to_print(&diagnostics, &ready, &mut progress),
            ["DSD-1: not on the bus"]
        );
    }
}
