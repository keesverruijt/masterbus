//! Signal K sidecar for Mastervolt MasterBus.
//!
//! Subscribes to the monitoring values of every device on the bus and serves
//! **Signal K deltas** as newline-delimited JSON over **TCP**. It listens on
//! `0.0.0.0:3009` by default; a Signal K server connects to it as a client
//! (data connection type: Signal K, over TCP).
//!
//! ```text
//! masterbus-signalk [listen-addr] [cache-dir]
//! ```
//!
//! Transport (USB / SocketCAN) and master role come from the per-host config
//! file (see `masterbus::FileConfig`); the file is created on first run.
//!
//! The MasterBus-field → Signal K-path mapping (and unit conversion to SI) lives
//! in [`map_field`]; it currently covers batteries and the CombiMaster and is
//! easy to extend per device class.
//!
//! # Which fields are published
//!
//! If the `MAPPING` environment variable points at a file, it gates output per
//! `<instance>.<menu>[.<group>]`. New devices are auto-added (menu-level = off;
//! the battery `cluster` group = on) and the file is rewritten; edit the
//! `true`/`false` flags while the service is stopped. Without `MAPPING`, every
//! mapped field is published.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use masterbus::{Config, DeviceId, FieldId, MasterBus, Menu, Value};
use serde_json::json;

/// Default TCP listen address.
const DEFAULT_LISTEN: &str = "0.0.0.0:3009";

/// The menu the sidecar publishes (only monitoring carries mapped data today).
const MENU: &str = "monitoring";

/// How often each value is (re)emitted.
const RATE: Duration = Duration::from_millis(1000);

fn main() {
    let mut args = std::env::args().skip(1);
    let listen = args.next().unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let config = Config { cache_path: args.next().map(Into::into), ..Default::default() };
    let mapping = std::env::var_os("MAPPING").map(PathBuf::from);

    let bus = match MasterBus::auto(config) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("masterbus-signalk: connect failed: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(bus, &listen, mapping.as_deref()) {
        eprintln!("masterbus-signalk: {e}");
        std::process::exit(1);
    }
}

/// A discovered field and where it lives (used to build the mapping + metadata).
struct FieldRec {
    device: DeviceId,
    index: FieldId,
    class: String,
    instance: String,
    group: String,
    name: String,
    unit: String,
}

/// Per-field metadata captured at startup so updates can be mapped cheaply.
struct FieldMeta {
    class: String,
    instance: String,
    name: String,
    unit: String,
}

/// Parse a mapping file (`<instance>.<menu>[.<group>] = true|false`, `#` comments).
fn load_mapping(path: &Path) -> BTreeMap<String, bool> {
    let mut map = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else { return map };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let on = matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on");
            map.insert(k.trim().to_string(), on);
        }
    }
    map
}

/// Rewrite the mapping file: a per-instance comment listing its groups, the
/// menu-level toggle, then any group-level toggles present in `map`.
fn save_mapping(
    path: &Path,
    map: &BTreeMap<String, bool>,
    groups_by_instance: &BTreeMap<String, BTreeSet<String>>,
) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str(
        "# masterbus-signalk Signal K mapping.\n\
         # Edit the true/false flags below while the service is STOPPED, then restart.\n\
         # Keys: <instance>.<menu>[.<group>] = true|false  (a group line overrides the\n\
         # menu line). New devices are added automatically: the menu-level toggle\n\
         # defaults to false and the battery `cluster` group to true.\n\n",
    );
    for (instance, groups) in groups_by_instance {
        let glist = groups.iter().cloned().collect::<Vec<_>>().join(", ");
        out.push_str(&format!("# {instance} \u{2014} groups: {glist}\n"));
        let mk = format!("{instance}.{MENU}");
        out.push_str(&format!("{mk} = {}\n", map.get(&mk).copied().unwrap_or(false)));
        for g in groups {
            let gk = format!("{instance}.{MENU}.{g}");
            if let Some(&v) = map.get(&gk) {
                out.push_str(&format!("{gk} = {v}\n"));
            }
        }
        out.push('\n');
    }
    std::fs::write(path, out)
}

/// Resolve whether a field's (instance, menu, group) is enabled: a group-level
/// entry wins over a menu-level one; the default is on only for `cluster`.
fn enabled(map: &BTreeMap<String, bool>, instance: &str, menu: &str, group: &str) -> bool {
    if let Some(&v) = map.get(&format!("{instance}.{menu}.{group}")) {
        return v;
    }
    if let Some(&v) = map.get(&format!("{instance}.{menu}")) {
        return v;
    }
    group == "cluster"
}

fn run(bus: MasterBus, listen: &str, mapping_path: Option<&Path>) -> std::io::Result<()> {
    // TCP server: clients (e.g. a Signal K server) connect and receive the delta
    // stream. The listener thread appends new connections to the shared set.
    let listener = TcpListener::bind(listen)?;
    eprintln!("masterbus-signalk: listening on {} (Signal K delta, ndjson)", listener.local_addr()?);
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let clients = clients.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                eprintln!("masterbus-signalk: client connected: {:?}", stream.peer_addr().ok());
                clients.lock().unwrap().push(stream);
            }
        });
    }

    // Discover the monitoring menu of every device, recording each field with its
    // (sanitized) group so the mapping file can gate it.
    let devices = bus.devices_all();
    let mut fields: Vec<FieldRec> = Vec::new();
    let mut groups_by_instance: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for dev in &devices {
        let name = dev.name().unwrap_or_default();
        let class = name.split_whitespace().next().unwrap_or("").to_string();
        // Instance id = the device name without its leading class word (already
        // implied by the SK path category), lowercased/sanitized.
        let label = name.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
        let instance = if !label.is_empty() {
            sanitize(&label)
        } else if !name.is_empty() {
            sanitize(&name)
        } else {
            dev.id().to_string()
        };

        let Ok(groups) = dev.tab(Menu::Monitoring) else { continue };
        for group in groups {
            let gname = sanitize(&group.name().unwrap_or_default());
            groups_by_instance.entry(instance.clone()).or_default().insert(gname.clone());
            for field in group.fields().unwrap_or_default() {
                fields.push(FieldRec {
                    device: dev.id(),
                    index: field.index(),
                    class: class.clone(),
                    instance: instance.clone(),
                    group: gname.clone(),
                    name: field.name().unwrap_or_default(),
                    unit: field.unit().unwrap_or_default(),
                });
            }
        }
    }

    // Load / auto-fill / rewrite the mapping file (if configured).
    let mut mapping = mapping_path.map(load_mapping).unwrap_or_default();
    if let Some(path) = mapping_path {
        use std::collections::btree_map::Entry;
        let mut added = false;
        for (instance, groups) in &groups_by_instance {
            // Menu-level toggle defaults off.
            if let Entry::Vacant(e) = mapping.entry(format!("{instance}.{MENU}")) {
                e.insert(false);
                added = true;
            }
            // The battery cluster group defaults on.
            if groups.contains("cluster") {
                if let Entry::Vacant(e) = mapping.entry(format!("{instance}.{MENU}.cluster")) {
                    e.insert(true);
                    added = true;
                }
            }
        }
        if added || !path.exists() {
            if let Err(e) = save_mapping(path, &mapping, &groups_by_instance) {
                eprintln!("masterbus-signalk: could not write {}: {e}", path.display());
            }
        }
    }

    // Build the emit metadata + per-device subscription list for enabled fields.
    let gated = mapping_path.is_some();
    let mut meta: HashMap<(DeviceId, FieldId), FieldMeta> = HashMap::new();
    let mut per_device: HashMap<DeviceId, Vec<FieldId>> = HashMap::new();
    for f in &fields {
        let on = !gated || enabled(&mapping, &f.instance, MENU, &f.group);
        if on {
            meta.insert(
                (f.device, f.index),
                FieldMeta { class: f.class.clone(), instance: f.instance.clone(), name: f.name.clone(), unit: f.unit.clone() },
            );
            per_device.entry(f.device).or_default().push(f.index);
        }
    }
    let mut subs = Vec::new();
    for (device, indices) in per_device {
        subs.push(bus.subscribe(device, indices, RATE, false));
    }

    eprintln!(
        "masterbus-signalk: streaming {} of {} fields from {} device(s){}",
        meta.len(),
        fields.len(),
        devices.len(),
        if gated { " (mapping-gated)" } else { "" },
    );

    loop {
        // Skip building deltas when nobody is listening (the channels are still
        // drained below so they don't grow unbounded).
        let have_clients = !clients.lock().unwrap().is_empty();
        let mut batch: Vec<u8> = Vec::new();
        for sub in &subs {
            // Coalesce to the latest value per path this cycle: a field can be
            // updated many times between polls (the boat's real masters poll some
            // values rapidly, and we emit those too).
            let mut latest: HashMap<String, serde_json::Value> = HashMap::new();
            while let Some(u) = sub.try_recv() {
                if have_clients
                    && let Some(m) = meta.get(&(u.device, u.field))
                    && let Some((path, value)) = map_field(&m.class, &m.instance, &m.name, &m.unit, &u.value)
                {
                    latest.insert(path, value);
                }
            }
            if !latest.is_empty() {
                let values: Vec<_> =
                    latest.into_iter().map(|(path, value)| json!({ "path": path, "value": value })).collect();
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

/// Map a (device-class, field) pair to a Signal K path and SI value.
///
/// Returns `None` for fields without a mapping (they are simply not emitted).
/// Extend per device class; the matched names/units are exactly those the device
/// reports for its monitoring fields.
fn map_field(
    class: &str,
    id: &str,
    name: &str,
    unit: &str,
    value: &Value,
) -> Option<(String, serde_json::Value)> {
    let celsius = match value {
        Value::Float(x) if x.is_finite() => Some(*x as f64),
        _ => None,
    };
    let float = celsius;
    let boolean = match value {
        Value::Boolean(b) => Some(*b),
        _ => None,
    };
    let seconds = match value {
        Value::Time(t) => {
            Some(t.days as f64 * 86400.0 + t.hour as f64 * 3600.0 + t.min as f64 * 60.0 + t.sec as f64)
        }
        _ => None,
    };
    let num = |v: f64| serde_json::Value::from(v);

    match class {
        // Battery monitors → electrical.batteries.<id>
        "BAT" => {
            let b = format!("electrical.batteries.{id}");
            match (name, unit) {
                ("State of charge", _) => float.map(|v| (format!("{b}.capacity.stateOfCharge"), num(v / 100.0))),
                ("Battery", "V") => float.map(|v| (format!("{b}.voltage"), num(v))),
                ("Battery", "A") => float.map(|v| (format!("{b}.current"), num(v))),
                ("Battery", "\u{b0}C") => celsius.map(|c| (format!("{b}.temperature"), num(c + 273.15))),
                ("Time remaining", _) => seconds.map(|s| (format!("{b}.capacity.timeRemaining"), num(s))),
                ("Cap. consumed", _) => float.map(|ah| (format!("{b}.capacity.dischargeSinceFull"), num(ah * 3600.0))),
                _ => None,
            }
        }
        // CombiMaster (inverter/charger) → electrical.inverters/chargers.<id>
        "CMR" => {
            let inv = format!("electrical.inverters.{id}");
            let chg = format!("electrical.chargers.{id}");
            match (name, unit) {
                ("Battery voltage", "V") => float.map(|v| (format!("{inv}.dc.voltage"), num(v))),
                ("Battery current", "A") => float.map(|v| (format!("{inv}.dc.current"), num(v))),
                ("Battery temp.", "\u{b0}C") => celsius.map(|c| (format!("{inv}.dc.temperature"), num(c + 273.15))),
                ("Output voltage", "V") => float.map(|v| (format!("{inv}.ac.voltage"), num(v))),
                ("Output power", "W") => float.map(|v| (format!("{inv}.ac.power"), num(v))),
                ("Output frequency", "Hz") => float.map(|v| (format!("{inv}.ac.frequency"), num(v))),
                ("Input voltage", "V") => float.map(|v| (format!("{chg}.acin.voltage"), num(v))),
                ("Input current", "A") => float.map(|v| (format!("{chg}.acin.current"), num(v))),
                ("Input frequency", "Hz") => float.map(|v| (format!("{chg}.acin.frequency"), num(v))),
                ("AC IN limit", "A") => float.map(|v| (format!("{chg}.acin.currentLimit"), num(v))),
                ("Inverter", _) => boolean.map(|b| (format!("{inv}.enabled"), serde_json::Value::Bool(b))),
                ("Charger", _) => boolean.map(|b| (format!("{chg}.enabled"), serde_json::Value::Bool(b))),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Lowercase and keep only Signal K path-segment-safe characters in an instance
/// id (lowercase reads more idiomatically in Signal K paths).
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if cleaned.is_empty() { "0".into() } else { cleaned }
}

/// Current UTC time as an ISO-8601 / RFC-3339 string (no date dependency).
fn now_rfc3339() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let (y, mo, day) = civil_from_days((secs / 86400) as i64);
    format!("{y:04}-{mo:02}-{day:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}
