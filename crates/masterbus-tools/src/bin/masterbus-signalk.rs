//! Signal K sidecar for Mastervolt MasterBus.
//!
//! Subscribes to the monitoring values of every device on the bus and serves
//! **Signal K deltas** as newline-delimited JSON over **TCP**. It listens on
//! `0.0.0.0:3009` by default; a Signal K server connects to it as a client
//! (data connection type: Signal K, over TCP).
//!
//! ```text
//! masterbus-signalk [listen-addr]
//! ```
//!
//! Transport (USB / SocketCAN), master role, and the schema cache directory
//! all come from the per-host config file (see `masterbus::FileConfig`); the
//! file is created on first run.
//!
//! The MasterBus-field → Signal K-path mapping (and unit conversion to SI) lives
//! in [`map_field`]; it currently covers batteries, the CombiMaster, the MAC
//! DC-DC charger, and the APR alternator regulator, and is easy to extend per
//! device class.
//!
//! # Which fields are published
//!
//! If the `MAPPING` environment variable points at a file, it gates output per
//! `<instance>.<menu>[.<group>]`. New devices are auto-added (menu-level = off;
//! the battery `cluster` group = on) and the file is rewritten; edit the
//! `true`/`false` flags while the service is stopped. Without `MAPPING`, every
//! mapped field is published.
//!
//! Besides live values, each published device also emits static `name` and
//! `manufacturer` (name + article/model) metadata once per client connection —
//! see [`static_meta_batch`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let listen = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let mapping = std::env::var_os("MAPPING").map(PathBuf::from);

    let bus = match MasterBus::auto(Config::default()) {
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

/// Per-device identity captured at startup, used to publish the static Signal K
/// `name` / `manufacturer` metadata for each device's node(s).
struct DeviceMetaRec {
    device: DeviceId,
    class: String,
    instance: String,
    /// Human-readable device name (as configured on the Mastervolt system).
    name: String,
    /// Article number → Signal K `manufacturer.model`.
    article: String,
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
    let Ok(text) = std::fs::read_to_string(path) else {
        return map;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let on = matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            );
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
        out.push_str(&format!(
            "{mk} = {}\n",
            map.get(&mk).copied().unwrap_or(false)
        ));
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

    // Discover the monitoring menu of every device, recording each field with its
    // (sanitized) group so the mapping file can gate it.
    let devices = bus.devices_all();
    let mut fields: Vec<FieldRec> = Vec::new();
    let mut device_metas: Vec<DeviceMetaRec> = Vec::new();
    let mut groups_by_instance: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for dev in &devices {
        let name = dev.name().unwrap_or_default();
        let class = name.split_whitespace().next().unwrap_or("").to_string();
        // Instance id = the device name without its leading class word (already
        // implied by the SK path category), lowercased/sanitized.
        let label = name
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        let instance = if !label.is_empty() {
            sanitize(&label)
        } else if !name.is_empty() {
            sanitize(&name)
        } else {
            dev.id().to_string()
        };

        device_metas.push(DeviceMetaRec {
            device: dev.id(),
            class: class.clone(),
            instance: instance.clone(),
            name: name.clone(),
            article: dev.article_number().unwrap_or_default(),
        });

        let Ok(groups) = dev.tab(Menu::Monitoring) else {
            continue;
        };
        for group in groups {
            let gname = sanitize(&group.name().unwrap_or_default());
            groups_by_instance
                .entry(instance.clone())
                .or_default()
                .insert(gname.clone());
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
                FieldMeta {
                    class: f.class.clone(),
                    instance: f.instance.clone(),
                    name: f.name.clone(),
                    unit: f.unit.clone(),
                },
            );
            per_device.entry(f.device).or_default().push(f.index);
        }
    }
    // Devices with at least one published field carry SK nodes; those are the
    // ones whose static name/manufacturer metadata is worth emitting.
    let published: HashSet<DeviceId> = per_device.keys().copied().collect();
    let mut subs = Vec::new();
    for (device, indices) in per_device {
        subs.push(bus.subscribe(device, indices, RATE, false));
    }

    // Render the static metadata batch and hand it to the accept thread (for
    // future clients) and to any client already connected during discovery.
    let sb = static_meta_batch(&device_metas, &published);
    *static_batch.lock().unwrap() = sb.clone();
    if !sb.is_empty() {
        let mut cs = clients.lock().unwrap();
        cs.retain_mut(|c| c.write_all(&sb).and_then(|()| c.flush()).is_ok());
    }

    eprintln!(
        "masterbus-signalk: streaming {} of {} fields from {} device(s){}",
        meta.len(),
        fields.len(),
        devices.len(),
        if gated { " (mapping-gated)" } else { "" },
    );

    // Paths whose unit `meta` has already been published. Meta is emitted inline
    // the first time a path is seen and also appended to `static_batch` so later
    // clients receive it on connect.
    let mut meta_sent: HashSet<String> = HashSet::new();
    loop {
        // Skip building deltas when nobody is listening (the channels are still
        // drained below so they don't grow unbounded).
        let have_clients = !clients.lock().unwrap().is_empty();
        let mut batch: Vec<u8> = Vec::new();
        let mut new_meta: Vec<serde_json::Value> = Vec::new();
        for sub in &subs {
            // Coalesce to the latest value per path this cycle: a field can be
            // updated many times between polls (the boat's real masters poll some
            // values rapidly, and we emit those too).
            let mut latest: HashMap<String, serde_json::Value> = HashMap::new();
            while let Some(u) = sub.try_recv() {
                if have_clients
                    && let Some(m) = meta.get(&(u.device, u.field))
                    && let Some((path, value)) =
                        map_field(&m.class, &m.instance, &m.name, &m.unit, &u.value)
                {
                    latest.insert(path, value);
                }
            }
            if !latest.is_empty() {
                // First sighting of a path → publish its unit metadata once.
                for path in latest.keys() {
                    if !meta_sent.contains(path)
                        && let Some(units) = sk_units(path)
                    {
                        new_meta.push(json!({ "path": path, "value": { "units": units } }));
                        meta_sent.insert(path.clone());
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

/// SI unit for a published Signal K path, keyed on its leaf segment. Signal K's
/// own metadata already carries units for the standard leaves (`voltage`,
/// `temperature`, …), but our non-standard nested leaves (`battery.temperature`,
/// `field.current`, `input.voltage`, `voltageSense`, …) are unknown to the
/// server, so it can't unit-convert them without a `meta` delta. We publish meta
/// for *every* known leaf (re-affirming the standard ones is harmless); `None`
/// leaves (`chargingMode`, `deviceMode`, `enabled`, `name`) carry no unit.
fn sk_units(path: &str) -> Option<&'static str> {
    Some(match path.rsplit('.').next().unwrap_or("") {
        "stateOfCharge" => "ratio",
        "timeRemaining" => "s",
        "dischargeSinceFull" => "C",
        "temperature" => "K",
        "voltage" | "voltageSense" => "V",
        "current" | "currentLimit" => "A",
        "power" => "W",
        "frequency" | "revolutions" => "Hz",
        _ => return None,
    })
}

/// Signal K base path(s) for a device class — the node(s) that carry this
/// device's values, and thus its static `name` / `manufacturer` metadata. The
/// CombiMaster spans two categories. Keep this in sync with [`map_field`]'s
/// per-class path prefixes.
fn sk_bases(class: &str, id: &str) -> Vec<String> {
    match class {
        "BAT" => vec![format!("electrical.batteries.{id}")],
        "CMR" => vec![
            format!("electrical.inverters.{id}"),
            format!("electrical.chargers.{id}"),
        ],
        "MAC" => vec![format!("electrical.chargers.{id}")],
        "APR" => vec![format!("electrical.alternators.{id}")],
        _ => vec![],
    }
}

/// Build the one-shot Signal K metadata batch: for every published device with a
/// mapped category, its `name` and `manufacturer` (name + model). Values are
/// static, so this is emitted once per client rather than on the poll loop.
fn static_meta_batch(devs: &[DeviceMetaRec], published: &HashSet<DeviceId>) -> Vec<u8> {
    let mut batch = Vec::new();
    for d in devs {
        if !published.contains(&d.device) {
            continue;
        }
        let mut values = Vec::new();
        for base in sk_bases(&d.class, &d.instance) {
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
        Value::Time(t) => Some(
            t.days as f64 * 86400.0 + t.hour as f64 * 3600.0 + t.min as f64 * 60.0 + t.sec as f64,
        ),
        _ => None,
    };
    // Selected label of a list/enum field (e.g. the charge-state name), lowercased.
    let list_label = value.label().map(|s| s.to_ascii_lowercase());
    let num = |v: f64| serde_json::Value::from(v);
    let text = |s: String| serde_json::Value::String(s);

    match class {
        // Battery monitors → electrical.batteries.<id>
        "BAT" => {
            let b = format!("electrical.batteries.{id}");
            match (name, unit) {
                ("State of charge", _) => {
                    float.map(|v| (format!("{b}.capacity.stateOfCharge"), num(v / 100.0)))
                }
                ("Battery", "V") => float.map(|v| (format!("{b}.voltage"), num(v))),
                ("Battery", "A") => float.map(|v| (format!("{b}.current"), num(v))),
                ("Battery", "\u{b0}C") => {
                    celsius.map(|c| (format!("{b}.temperature"), num(c + 273.15)))
                }
                ("Time remaining", _) => {
                    seconds.map(|s| (format!("{b}.capacity.timeRemaining"), num(s)))
                }
                ("Cap. consumed", _) => {
                    float.map(|ah| (format!("{b}.capacity.dischargeSinceFull"), num(ah * 3600.0)))
                }
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
                ("Battery temp.", "\u{b0}C") => {
                    celsius.map(|c| (format!("{inv}.dc.temperature"), num(c + 273.15)))
                }
                ("Output voltage", "V") => float.map(|v| (format!("{inv}.ac.voltage"), num(v))),
                ("Output power", "W") => float.map(|v| (format!("{inv}.ac.power"), num(v))),
                ("Output frequency", "Hz") => {
                    float.map(|v| (format!("{inv}.ac.frequency"), num(v)))
                }
                ("Input voltage", "V") => float.map(|v| (format!("{chg}.acin.voltage"), num(v))),
                ("Input current", "A") => float.map(|v| (format!("{chg}.acin.current"), num(v))),
                ("Input frequency", "Hz") => {
                    float.map(|v| (format!("{chg}.acin.frequency"), num(v)))
                }
                ("AC IN limit", "A") => float.map(|v| (format!("{chg}.acin.currentLimit"), num(v))),
                ("Inverter", _) => {
                    boolean.map(|b| (format!("{inv}.enabled"), serde_json::Value::Bool(b)))
                }
                ("Charger", _) => {
                    boolean.map(|b| (format!("{chg}.enabled"), serde_json::Value::Bool(b)))
                }
                _ => None,
            }
        }
        // MAC — DC-DC battery charger (e.g. "MAC Plus 12/24"): a DC source on the
        // input steps up/down to charge the battery on the output. Canonical
        // charger `voltage`/`current`/`temperature` describe the output (battery)
        // side; the DC input side hangs off `.input.*`.
        //
        // Unlike the other classes, the MAC schema leaves the unit empty on some
        // monitoring fields (observed on MAC Plus 12/24: output voltage, output
        // current, input current, battery voltage sense), so a strict (name,
        // unit) match drops them silently. Those arms accept the unit *or* the
        // empty string. The rest keep the strict match on purpose: "Device" and
        // "Battery" are generic names that only °C tells apart, and wildcarding
        // them would publish any same-named float as a kelvin temperature.
        "MAC" => {
            let chg = format!("electrical.chargers.{id}");
            match (name, unit.trim()) {
                ("Output voltage", "V" | "") => float.map(|v| (format!("{chg}.voltage"), num(v))),
                ("Output current", "A" | "") => float.map(|v| (format!("{chg}.current"), num(v))),
                ("Input voltage", "V" | "") => {
                    float.map(|v| (format!("{chg}.input.voltage"), num(v)))
                }
                ("Input current", "A" | "") => {
                    float.map(|v| (format!("{chg}.input.current"), num(v)))
                }
                ("Bat. volt sense", "V" | "") => {
                    float.map(|v| (format!("{chg}.voltageSense"), num(v)))
                }
                ("Device", "\u{b0}C") => {
                    celsius.map(|c| (format!("{chg}.temperature"), num(c + 273.15)))
                }
                ("Battery", "\u{b0}C") => {
                    celsius.map(|c| (format!("{chg}.battery.temperature"), num(c + 273.15)))
                }
                // "Device state" (Standby/Charging/Fault/…) → deviceMode: the
                // device-level state, orthogonal to the charge stage below.
                ("Device state", _) => list_label.map(|s| (format!("{chg}.deviceMode"), text(s))),
                // "Charge state" (Bulk/Absorption/Float/…) → chargingMode.
                ("Charge state", _) => list_label.map(|s| (format!("{chg}.chargingMode"), text(s))),
                // "Standby" off = charger active.
                ("Standby", _) => {
                    boolean.map(|b| (format!("{chg}.enabled"), serde_json::Value::Bool(!b)))
                }
                _ => None,
            }
        }
        // APR — Alpha Pro alternator regulator ("APR Alternator"): a
        // mechanically-driven alternator plus an external shunt/battery monitor.
        // Canonical alternator `voltage`/`temperature`/`revolutions` describe the
        // alternator; the battery it charges (sensed both directly and via the
        // shunt) hangs off `.battery.*`, the engine drive off `.engine.*`.
        //
        // Note: "Battery voltage"/"Battery temp." occur in both the Battery and
        // Shunt groups (same name+unit, different field index); since `map_field`
        // sees no group they share one path and coalesce to the last sample of
        // the cycle — harmless, they read the same battery.
        "APR" => {
            let alt = format!("electrical.alternators.{id}");
            match (name, unit) {
                ("Alternator volt.", "V") => float.map(|v| (format!("{alt}.voltage"), num(v))),
                ("Sense voltage", "V") => float.map(|v| (format!("{alt}.voltageSense"), num(v))),
                ("Field current", "A") => float.map(|v| (format!("{alt}.field.current"), num(v))),
                ("Alternator temp.", "\u{b0}C") => {
                    celsius.map(|c| (format!("{alt}.temperature"), num(c + 273.15)))
                }
                // Shaft speeds → revolutions (Signal K wants Hz, i.e. rpm / 60).
                ("Alternator shaft", "rpm") => {
                    float.map(|r| (format!("{alt}.revolutions"), num(r / 60.0)))
                }
                ("Engine shaft", "rpm") => {
                    float.map(|r| (format!("{alt}.engine.revolutions"), num(r / 60.0)))
                }
                // "Charger state" (Off/Bulk/Absorption/Float/…) → chargingMode.
                ("Charger state", _) => {
                    list_label.map(|s| (format!("{alt}.chargingMode"), text(s)))
                }
                // Battery being charged (direct sense + external shunt).
                ("State of charge", "%") => {
                    float.map(|v| (format!("{alt}.battery.stateOfCharge"), num(v / 100.0)))
                }
                ("Battery voltage", "V") => {
                    float.map(|v| (format!("{alt}.battery.voltage"), num(v)))
                }
                ("Battery current", "A") => {
                    float.map(|v| (format!("{alt}.battery.current"), num(v)))
                }
                ("Battery temp.", "\u{b0}C") => {
                    celsius.map(|c| (format!("{alt}.battery.temperature"), num(c + 273.15)))
                }
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
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "0".into()
    } else {
        cleaned
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `map_field` for a float monitoring value.
    fn f(class: &str, name: &str, unit: &str, v: f32) -> Option<(String, serde_json::Value)> {
        map_field(class, "1", name, unit, &Value::Float(v))
    }

    /// The path `map_field` produced, for assertions that only care about routing.
    fn path(r: Option<(String, serde_json::Value)>) -> Option<String> {
        r.map(|(p, _)| p)
    }

    /// The MAC schema reports an empty unit on several monitoring fields; they
    /// must still map, onto the same paths as when the unit is present.
    #[test]
    fn mac_maps_fields_with_a_missing_unit() {
        for (name, unit) in [
            ("Output voltage", "V"),
            ("Output current", "A"),
            ("Input current", "A"),
            ("Bat. volt sense", "V"),
        ] {
            assert_eq!(
                path(f("MAC", name, unit, 1.0)),
                path(f("MAC", name, "", 1.0)),
                "{name}"
            );
            assert!(
                path(f("MAC", name, "", 1.0)).is_some(),
                "{name} dropped with an empty unit"
            );
        }
    }

    /// The MAC output is the battery side, and lands on the canonical Signal K
    /// charger leaves — not on nested `output.*` / `device.*` ones the server
    /// has no metadata for.
    #[test]
    fn mac_publishes_canonical_charger_paths() {
        assert_eq!(
            path(f("MAC", "Output voltage", "", 27.4)).as_deref(),
            Some("electrical.chargers.1.voltage")
        );
        assert_eq!(
            path(f("MAC", "Output current", "", 42.0)).as_deref(),
            Some("electrical.chargers.1.current")
        );
        assert_eq!(
            path(f("MAC", "Device", "\u{b0}C", 20.0)).as_deref(),
            Some("electrical.chargers.1.temperature")
        );
    }

    /// Relaxing the unit match must not leak to the temperature fields: "Device"
    /// and "Battery" are generic names that only °C tells apart, so a same-named
    /// field in another unit must not be published as a kelvin temperature.
    #[test]
    fn mac_temperatures_still_require_degrees_celsius() {
        assert_eq!(f("MAC", "Device", "", 20.0), None);
        assert_eq!(f("MAC", "Battery", "V", 12.8), None);
        assert_eq!(f("MAC", "Battery", "A", 3.0), None);
    }

    /// Celsius → kelvin on the way out.
    #[test]
    fn mac_temperature_is_converted_to_kelvin() {
        let (_, v) = f("MAC", "Battery", "\u{b0}C", 25.0).unwrap();
        assert_eq!(v.as_f64().unwrap(), 298.15);
    }

    /// "Standby" is the inverse of Signal K's `enabled`.
    #[test]
    fn mac_standby_inverts_into_enabled() {
        let on = map_field("MAC", "1", "Standby", "", &Value::Boolean(false));
        assert_eq!(
            on,
            Some((
                "electrical.chargers.1.enabled".into(),
                serde_json::Value::Bool(true)
            ))
        );
        let off = map_field("MAC", "1", "Standby", "", &Value::Boolean(true));
        assert_eq!(
            off,
            Some((
                "electrical.chargers.1.enabled".into(),
                serde_json::Value::Bool(false)
            ))
        );
    }

    /// Enum fields publish their lowercased label.
    #[test]
    fn mac_enum_states_publish_their_label() {
        let list = |i: i32| Value::List {
            index: i,
            options: vec!["Off".into(), "Bulk".into()],
        };
        assert_eq!(
            map_field("MAC", "1", "Charge state", "", &list(1)),
            Some((
                "electrical.chargers.1.chargingMode".into(),
                serde_json::Value::String("bulk".into())
            ))
        );
        assert_eq!(
            path(map_field("MAC", "1", "Device state", "", &list(0))).as_deref(),
            Some("electrical.chargers.1.deviceMode")
        );
    }

    /// The other classes keep the strict (name, unit) match — BAT reports three
    /// different quantities all named "Battery", told apart only by their unit.
    #[test]
    fn other_classes_still_disambiguate_on_unit() {
        assert_eq!(
            path(f("BAT", "Battery", "V", 12.8)).as_deref(),
            Some("electrical.batteries.1.voltage")
        );
        assert_eq!(
            path(f("BAT", "Battery", "A", -5.0)).as_deref(),
            Some("electrical.batteries.1.current")
        );
        assert_eq!(
            path(f("BAT", "Battery", "\u{b0}C", 20.0)).as_deref(),
            Some("electrical.batteries.1.temperature")
        );
        assert_eq!(f("BAT", "Battery", "", 12.8), None);
    }

    /// Every leaf MAC publishes a number on must carry unit metadata, or Signal
    /// K cannot unit-convert it.
    #[test]
    fn mac_numeric_leaves_have_unit_metadata() {
        for (name, unit) in [
            ("Output voltage", ""),
            ("Output current", ""),
            ("Input voltage", "V"),
            ("Input current", ""),
            ("Bat. volt sense", ""),
            ("Device", "\u{b0}C"),
            ("Battery", "\u{b0}C"),
        ] {
            let p = path(f("MAC", name, unit, 1.0)).unwrap();
            assert!(sk_units(&p).is_some(), "{p} has no unit metadata");
        }
    }
}
