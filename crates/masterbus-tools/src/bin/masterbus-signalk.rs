//! Signal K sidecar for Mastervolt MasterBus.
//!
//! Subscribes to the fields a curated mapping names and serves **Signal K
//! deltas** as newline-delimited JSON over **TCP**. It listens on
//! `0.0.0.0:3009` by default; a Signal K server connects to it as a client
//! (data connection type: Signal K, over TCP).
//!
//! ```text
//! masterbus-signalk [listen-addr]
//! ```
//!
//! Transport (USB / SocketCAN), master role, the schema cache directory and
//! the default listen address all come from the per-host config file (see
//! `masterbus::FileConfig`); the file is created on first run. A listen
//! address given on the command line overrides the file.
//!
//! # What gets published
//!
//! Exactly what `mapping.json` says, and nothing else. The file sits beside
//! `config.ini`; `MAPPING` overrides the location. It is keyed on device
//! **serial number** and **field id**, because those are what the firmware
//! fixes — device, group and field *names* are installer-editable, and issue
//! #12 has the bus that proves matching on them cannot work.
//!
//! The file is meant to be curated by a human in `masterbus-tui`. When it is
//! missing or empty, this service seeds one from [`masterbus_tools::seed`] —
//! the bundled per-model database first, then the per-class name heuristics —
//! and writes it out, so an install that worked before keeps working and has
//! something to edit.
//!
//! Unit conversion and unit metadata are **derived** from the field's own
//! unit, never stored (see [`masterbus_tools::units::to_si`]); the target
//! leaf only cross-checks. A mapping entry whose units cannot be reconciled is
//! reported at startup and skipped rather than published as a wrong number.
//!
//! The mapping file is re-read when it changes on disk, so an edit in
//! `masterbus-tui --mapping` takes effect within a couple of seconds without
//! a restart.
//!
//! Besides live values, each published device also emits static `name` and
//! `manufacturer` metadata once per client connection.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use masterbus::{Config, DeviceId, FieldId, MasterBus, Menu, Subscription};
use masterbus_tools::mapping::{DeviceMapping, FieldMapping, Mapping, field_key, parse_field_key};
use masterbus_tools::seed;
use masterbus_tools::signalk::{self, Plan};
use serde_json::json;

/// Default TCP listen address.
const DEFAULT_LISTEN: &str = "0.0.0.0:3009";

/// The menu whose fields are offered for mapping. Configuration and Service
/// carry settings rather than measurements.
const MENU: Menu = Menu::Monitoring;

/// How often each value is (re)emitted.
const RATE: Duration = Duration::from_millis(1000);

/// How often the mapping file is checked for a change.
const RELOAD_CHECK: Duration = Duration::from_secs(2);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let file_config = masterbus::FileConfig::load_or_create().ok();
    // Listen address: command line wins, then `listen` in the per-host config
    // file, then the built-in default. Having it in the config file is what
    // lets the systemd unit drop its own environment file, so this project
    // keeps exactly one configuration directory per host.
    let listen = std::env::args().nth(1).unwrap_or_else(|| {
        file_config
            .as_ref()
            .and_then(|c| c.listen.clone())
            .unwrap_or_else(|| DEFAULT_LISTEN.to_string())
    });
    // Mapping file: `MAPPING` overrides, otherwise it sits beside config.ini.
    let mapping_path: Option<PathBuf> = std::env::var_os("MAPPING")
        .map(PathBuf::from)
        .or_else(|| file_config.as_ref().map(|c| c.mapping_path()));

    let bus = match MasterBus::auto(Config::default()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("masterbus-signalk: connect failed: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(bus, &listen, mapping_path.as_deref()) {
        eprintln!("masterbus-signalk: {e}");
        std::process::exit(1);
    }
}

/// One discovered device, reduced to what mapping needs.
struct DeviceRec {
    /// Bus address.
    id: DeviceId,
    /// Serial number — the mapping file's key for this unit.
    serial: String,
    /// Article (model) number.
    article: String,
    /// Installer-assigned name. Displayed and recorded, never matched on.
    name: String,
    /// Firmware version.
    firmware: String,
    /// Proposed Signal K instance id, used when seeding.
    instance: String,
    /// Monitoring fields as the device reports them.
    fields: Vec<FieldRec>,
}

/// One monitoring field, reduced to what mapping needs.
#[derive(Clone)]
struct FieldRec {
    id: FieldId,
    name: String,
    unit: String,
    /// Labels, for an enum; empty otherwise.
    options: Vec<String>,
}

/// Everything needed to turn one field's updates into a Signal K value.
struct Emit {
    /// Target Signal K path.
    path: String,
    /// How to get there, derived from the field's unit and the mapping entry.
    plan: Plan,
}

/// Walk the bus and collect every device's monitoring fields.
fn discover(bus: &MasterBus) -> Vec<DeviceRec> {
    let mut devices = bus.devices_all();
    devices.sort_by_key(|d| d.id());
    let mut out = Vec::new();
    for dev in &devices {
        let identity = dev
            .identity()
            .unwrap_or_else(|_| masterbus::DeviceIdentity {
                article: String::new(),
                serial: String::new(),
                revision: String::new(),
                name: String::new(),
                firmware: String::new(),
            });
        let mut fields = Vec::new();
        for group in dev.tab_info(MENU).unwrap_or_default() {
            for field in group.fields {
                fields.push(FieldRec {
                    id: field.index,
                    name: field.name,
                    unit: field.unit,
                    options: field.options,
                });
            }
        }
        out.push(DeviceRec {
            id: dev.id(),
            instance: seed::instance_of(&identity.name, dev.id()),
            serial: identity.serial,
            article: identity.article,
            name: identity.name,
            firmware: identity.firmware,
            fields,
        });
    }
    out
}

/// Build a mapping from the per-class name heuristics — the migration path for
/// an install that has no curated file yet, and the starting point a human
/// edits in the TUI.
fn seed_mapping(devices: &[DeviceRec]) -> Mapping {
    let mut m = Mapping::new();
    for d in devices {
        if d.serial.is_empty() {
            continue;
        }
        let class = seed::class_of(&d.name).to_string();
        let mut dm = DeviceMapping {
            article: d.article.clone(),
            firmware: d.firmware.clone(),
            name: d.name.clone(),
            instance: d.instance.clone(),
            ..Default::default()
        };
        // Two fields of one device that seed to the same path would just
        // coalesce to whichever arrives last, which is a mapping file that
        // silently disagrees with itself. Real devices do this: a battery
        // reports the same six measurements once for its cluster and once for
        // itself, and an alternator reports battery voltage in both its Battery
        // and Shunt groups. The first field id wins and the rest are left out
        // for a human to add deliberately if they want them.
        let mut taken: HashMap<String, FieldId> = HashMap::new();
        let mut ordered: Vec<_> = d.fields.iter().collect();
        ordered.sort_by_key(|f| f.id);
        for f in ordered {
            let Some((s, _tier)) = seed::suggest_best(
                &d.article,
                &d.firmware,
                &class,
                &d.instance,
                f.id,
                &f.name,
                &f.unit,
            ) else {
                continue;
            };
            if let Some(first) = taken.get(s.path.as_str()) {
                log::debug!(
                    "{}: {} would publish to {}, already taken by {}; skipped",
                    d.name,
                    field_key(f.id),
                    s.path,
                    field_key(*first)
                );
                continue;
            }
            taken.insert(s.path.clone(), f.id);
            dm.fields.insert(
                field_key(f.id),
                FieldMapping {
                    path: s.path,
                    invert: s.invert,
                    ..Default::default()
                },
            );
        }
        if !dm.fields.is_empty() {
            m.devices.insert(d.serial.clone(), dm);
        }
    }
    m
}

/// Resolve the mapping against the live bus: which (device, field) pairs to
/// subscribe to, and how to encode each one. Entries that cannot be honoured
/// are reported once at startup rather than failing silently.
fn resolve(devices: &[DeviceRec], mapping: &Mapping) -> HashMap<(DeviceId, FieldId), Emit> {
    let mut emit = HashMap::new();
    let by_serial: HashMap<&str, &DeviceRec> = devices
        .iter()
        .filter(|d| !d.serial.is_empty())
        .map(|d| (d.serial.as_str(), d))
        .collect();

    for (serial, dm) in &mapping.devices {
        let Some(dev) = by_serial.get(serial.as_str()) else {
            eprintln!(
                "masterbus-signalk: mapping lists serial {serial:?}, which is not on the bus"
            );
            continue;
        };
        if !dm.firmware.is_empty() && dm.firmware != dev.firmware {
            eprintln!(
                "masterbus-signalk: {} ({serial}) is running firmware {} but its mapping was \
                 written for {}; field ids may have moved — check it in `masterbus-tui --mapping`",
                dev.name, dev.firmware, dm.firmware
            );
        }
        for (key, fm) in &dm.fields {
            let Some(id) = parse_field_key(key) else {
                eprintln!("masterbus-signalk: {serial}: {key:?} is not a field id");
                continue;
            };
            let Some(f) = dev.fields.iter().find(|f| f.id == id) else {
                eprintln!(
                    "masterbus-signalk: {} ({serial}) has no monitoring field {key}",
                    dev.name
                );
                continue;
            };
            // The unit and conversion follow from the field's own unit; the
            // leaf only cross-checks. What cannot work is skipped here, with
            // the reason, rather than published as a wrong number.
            let plan = match signalk::plan(&fm.path, &f.unit, &f.options, fm) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "masterbus-signalk: {} ({serial}) {key} → {}: {e}; skipped — fix it in \
                         `masterbus-tui --mapping`",
                        dev.name, fm.path
                    );
                    continue;
                }
            };
            if let Some(w) = &plan.warning {
                eprintln!(
                    "masterbus-signalk: {} ({serial}) {key} → {}: {w}",
                    dev.name, fm.path
                );
            }
            emit.insert(
                (dev.id, id),
                Emit {
                    path: fm.path.clone(),
                    plan,
                },
            );
        }
    }
    emit
}

fn run(bus: MasterBus, listen: &str, mapping_path: Option<&Path>) -> std::io::Result<()> {
    // TCP server: clients (e.g. a Signal K server) connect and receive the delta
    // stream. The listener thread appends new connections to the shared set.
    let listener = TcpListener::bind(listen)?;
    eprintln!(
        "masterbus-signalk: listening on {} (Signal K delta, ndjson)",
        listener.local_addr()?
    );
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    // Static per-device metadata (name / manufacturer), rendered once discovery
    // completes. Replayed to every client the moment it connects so late joiners
    // still learn each device's identity without waiting for a value change.
    let static_batch: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let clients = clients.clone();
        let static_batch = static_batch.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
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
                clients.lock().unwrap().push(stream);
            }
        });
    }

    let devices = discover(&bus);

    // Load the curated mapping; seed one on first run so there is something to
    // publish and, more importantly, something to edit.
    let mut mapping = match mapping_path {
        Some(p) => Mapping::load(p).unwrap_or_else(|e| {
            eprintln!("masterbus-signalk: {e}; starting from an empty mapping");
            Mapping::new()
        }),
        None => Mapping::new(),
    };
    if mapping.is_empty() {
        mapping = seed_mapping(&devices);
        match mapping_path {
            Some(p) if !mapping.is_empty() => match mapping.save(p) {
                Ok(()) => eprintln!(
                    "masterbus-signalk: no mapping yet — seeded {} field(s) from the built-in \
                     heuristics and wrote {}. Review it with `masterbus-tui --mapping`.",
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
    }

    let mut active = activate(&bus, &devices, &mapping, &clients, &static_batch);
    let mut stamp = mtime(mapping_path);
    let mut last_check = Instant::now();

    loop {
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
                        active = activate(&bus, &devices, &m, &clients, &static_batch);
                    }
                    Err(e) => eprintln!("masterbus-signalk: {e}; keeping the previous mapping"),
                }
            }
        }

        // Skip building deltas when nobody is listening (the channels are still
        // drained below so they don't grow unbounded).
        let have_clients = !clients.lock().unwrap().is_empty();
        let mut batch: Vec<u8> = Vec::new();
        let mut new_meta: Vec<serde_json::Value> = Vec::new();
        for sub in &active.subs {
            // Coalesce to the latest value per path this cycle: a field can be
            // updated many times between polls (the boat's real masters poll some
            // values rapidly, and we emit those too).
            let mut latest: HashMap<String, serde_json::Value> = HashMap::new();
            while let Some(u) = sub.try_recv() {
                if have_clients
                    && let Some(e) = active.emit.get(&(u.device, u.field))
                    && let Some(value) =
                        signalk::encode(&u.value, e.plan.conv, e.plan.invert, &e.plan.truth)
                {
                    latest.insert(e.path.clone(), value);
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
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The live state a mapping resolves to: what to publish, the subscriptions
/// feeding it, and the unit metadata each path carries. Rebuilt whole when
/// the mapping file changes; dropping the old one unsubscribes.
struct Active {
    emit: HashMap<(DeviceId, FieldId), Emit>,
    subs: Vec<Subscription>,
    /// Path → unit metadata, from the device units behind each path.
    units: HashMap<String, &'static str>,
    /// Paths whose unit `meta` has already been published this activation.
    meta_sent: HashSet<String>,
}

/// Resolve a mapping against the bus, subscribe to what it names, and
/// (re)publish the static per-device metadata.
fn activate(
    bus: &MasterBus,
    devices: &[DeviceRec],
    mapping: &Mapping,
    clients: &Mutex<Vec<TcpStream>>,
    static_batch: &Mutex<Vec<u8>>,
) -> Active {
    let emit = resolve(devices, mapping);
    let mut per_device: HashMap<DeviceId, Vec<FieldId>> = HashMap::new();
    for (device, field) in emit.keys() {
        per_device.entry(*device).or_default().push(*field);
    }
    let published: HashSet<DeviceId> = per_device.keys().copied().collect();
    let mut subs = Vec::new();
    for (device, indices) in per_device {
        subs.push(bus.subscribe(device, indices, RATE, false));
    }
    let units: HashMap<String, &'static str> = emit
        .values()
        .filter_map(|e| e.plan.unit.map(|u| (e.path.clone(), u)))
        .collect();

    // Render the static metadata batch and hand it to the accept thread (for
    // future clients) and to any client already connected.
    let sb = static_meta_batch(devices, &emit, &published);
    *static_batch.lock().unwrap() = sb.clone();
    if !sb.is_empty() {
        let mut cs = clients.lock().unwrap();
        cs.retain_mut(|c| c.write_all(&sb).and_then(|()| c.flush()).is_ok());
    }

    let total: usize = devices.iter().map(|d| d.fields.len()).sum();
    eprintln!(
        "masterbus-signalk: streaming {} of {total} monitoring fields from {} of {} device(s)",
        emit.len(),
        published.len(),
        devices.len(),
    );
    Active {
        emit,
        subs,
        units,
        meta_sent: HashSet::new(),
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
                })
                .collect(),
        }
    }

    /// The MLI Ultra field names that the old per-class table silently dropped.
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

    /// Two devices that both advertise as `CHG` with unrelated field sets. The
    /// class-and-name table maps neither; the article-keyed database maps both,
    /// differently, which is the case that motivated #12.
    #[test]
    fn seeding_tells_the_two_charger_articles_apart() {
        let mut mass = dev(
            "MASS-1",
            "CHG 24V Ch.U4-1",
            &[
                (0x00E, "Battery voltage", "V"),
                (0x00F, "Battery current", "A"),
            ],
        );
        mass.article = "40021006".into();
        mass.firmware = "7.9".into();
        // The renamed outputs from the boat in #6.
        let mut cm = dev(
            "CM-1",
            "CHG 12V ChargerE",
            &[(0x002, "Eng.batt", "V"), (0x004, "Gen.batt", "V")],
        );
        cm.article = "44010250".into();
        cm.firmware = "0.5".into();

        let m = seed_mapping(&[mass, cm]);
        assert_eq!(
            m.devices["MASS-1"].fields[&field_key(0x00E)].path,
            "electrical.chargers.24v-ch-u4-1.voltage"
        );
        assert_eq!(
            m.devices["CM-1"].fields[&field_key(0x002)].path,
            "electrical.chargers.12v-chargere.output.1.voltage"
        );
    }

    #[test]
    fn seeding_covers_the_battery_names_the_old_table_missed() {
        let m = seed_mapping(&[mli()]);
        let d = &m.devices["MLI-1"];
        assert_eq!(
            d.fields[&field_key(0x001)].path,
            "electrical.batteries.24v-service.voltage"
        );
        assert_eq!(
            d.fields[&field_key(0x005)].path,
            "electrical.batteries.24v-service.temperature"
        );
        // A relay has no Signal K home, so it is simply absent.
        assert!(!d.fields.contains_key(&field_key(0x022)));
    }

    /// Found by deploying onto a live boat: a battery reports the same six
    /// measurements once for its cluster and once for itself, so the seed
    /// produced two fields writing the same Signal K path. They would coalesce
    /// to whichever arrived last, giving a file that silently disagrees with
    /// itself. The lowest field id wins; the rest are left for a human to add
    /// deliberately.
    #[test]
    fn a_device_never_seeds_two_fields_onto_one_path() {
        let d = dev(
            "MLI-CLUSTER",
            "BAT Main Batt",
            &[
                // Cluster group.
                (0x000, "State of charge", "%"),
                (0x001, "Battery", "V"),
                (0x005, "Battery", "\u{b0}C"),
                // The device's own battery group: same measurements again.
                (0x088, "State of charge", "%"),
                (0x08B, "Battery", "V"),
                (0x08D, "Battery", "\u{b0}C"),
            ],
        );
        let m = seed_mapping(&[d]);
        let f = &m.devices["MLI-CLUSTER"].fields;
        let paths: Vec<&str> = f.values().map(|v| v.path.as_str()).collect();
        let unique: HashSet<&str> = paths.iter().copied().collect();
        assert_eq!(paths.len(), unique.len(), "duplicate paths: {paths:?}");
        // The lower id of each pair survives.
        assert!(f.contains_key(&field_key(0x001)));
        assert!(!f.contains_key(&field_key(0x08B)));
    }

    /// The alternator case, which the old code documented as harmless: battery
    /// voltage appears in both the Battery and Shunt groups.
    #[test]
    fn the_alternators_repeated_battery_reading_is_seeded_once() {
        let mut d = dev(
            "APR-1",
            "APR Alternator",
            &[
                (0x006, "Battery voltage", "V"),
                (0x014, "Battery voltage", "V"),
            ],
        );
        d.article = "45512000".into();
        let m = seed_mapping(&[d]);
        let f = &m.devices["APR-1"].fields;
        assert_eq!(f.len(), 1);
        assert!(f.contains_key(&field_key(0x006)));
    }

    #[test]
    fn seeding_records_identity_for_later_editing() {
        let m = seed_mapping(&[mli()]);
        let d = &m.devices["MLI-1"];
        assert_eq!(d.article, "66026000");
        assert_eq!(d.firmware, "2.14");
        assert_eq!(d.name, "BAT 24V Service");
        assert_eq!(d.instance, "24v-service");
    }

    #[test]
    fn a_device_with_no_serial_cannot_be_keyed_and_is_skipped() {
        let mut d = mli();
        d.serial = String::new();
        assert!(seed_mapping(&[d]).is_empty());
    }

    #[test]
    fn resolve_derives_the_conversion_from_the_units() {
        let devices = vec![mli()];
        let m = seed_mapping(&devices);
        let emit = resolve(&devices, &m);
        let id = devices[0].id;
        // Celsius into a kelvin leaf.
        let t = &emit[&(id, 0x005)];
        assert!((t.plan.conv.apply(20.0) - 293.15).abs() < 1e-9);
        assert_eq!(t.plan.unit, Some("K"));
        // Percent into a ratio leaf.
        let soc = &emit[&(id, 0x000)];
        assert!((soc.plan.conv.apply(87.0) - 0.87).abs() < 1e-9);
        // Volts into a volts leaf.
        assert!(emit[&(id, 0x001)].plan.conv.is_identity());
    }

    #[test]
    fn resolve_skips_entries_the_bus_cannot_honour() {
        let devices = vec![mli()];
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        // A field this device does not have.
        dm.fields.insert(
            field_key(0x0FF),
            FieldMapping {
                path: "electrical.batteries.x.voltage".into(),
                ..Default::default()
            },
        );
        // A field whose unit cannot reach the target leaf.
        dm.fields.insert(
            field_key(0x002),
            FieldMapping {
                path: "electrical.batteries.x.temperature".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        // An entire device that is not on the bus.
        m.devices.insert("GHOST".into(), DeviceMapping::default());

        let emit = resolve(&devices, &m);
        assert!(emit.is_empty(), "nothing publishable should survive");
    }

    #[test]
    fn nodes_come_from_the_paths_a_device_actually_uses() {
        let devices = vec![mli()];
        let m = seed_mapping(&devices);
        let emit = resolve(&devices, &m);
        assert_eq!(
            nodes_of(devices[0].id, &emit),
            vec!["electrical.batteries.24v-service".to_string()]
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
        let emit = resolve(&devices, &seed_mapping(&devices));
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
        let emit = resolve(&devices, &seed_mapping(&devices));
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
        let emit = resolve(&devices, &m);
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
        let emit = resolve(&devices, &m);
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
        let emit = resolve(&devices, &m);
        let id = devices[0].id;
        assert!(emit[&(id, 0x001)].plan.truth["Activated"]);
        assert!(!emit.contains_key(&(id, 0x002)), "Alarm is ambiguous");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
