//! Signal K sidecar for Mastervolt MasterBus.
//!
//! Subscribes to the monitoring values of every device on the bus and prints
//! **Signal K deltas** (one JSON object per line) to stdout. Point a Signal K
//! server "Execute" connection (data format: Signal K) at this binary:
//!
//! ```text
//! masterbus-signalk <can-interface> [cache-dir]
//! ```
//!
//! The MasterBus-field → Signal K-path mapping (and unit conversion to SI) lives
//! in [`map_field`]; it currently covers batteries and the CombiMaster and is
//! easy to extend per device class.

// The bus runtime is reachable only on Linux (SocketCAN); elsewhere the helpers
// type-check but are unused.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use masterbus::{Config, MasterBus, Menu, Value};
use serde_json::json;

/// How often each value is (re)emitted.
const RATE: Duration = Duration::from_millis(1000);

fn main() {
    let mut args = std::env::args().skip(1);
    let iface = match args.next() {
        Some(s) => s,
        None => {
            eprintln!("usage: masterbus-signalk <can-interface> [cache-dir]");
            std::process::exit(1);
        }
    };
    let config = Config { cache_path: args.next().map(Into::into), ..Default::default() };

    #[cfg(target_os = "linux")]
    {
        match MasterBus::socketcan(&iface, config) {
            Ok(bus) => {
                if let Err(e) = run(bus) {
                    eprintln!("masterbus-signalk: {e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("masterbus-signalk: connect failed: {e}");
                std::process::exit(2);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (iface, config);
        eprintln!("masterbus-signalk requires Linux/SocketCAN");
    }
}

/// Per-field metadata captured at startup so updates can be mapped cheaply.
struct FieldMeta {
    class: String,
    instance: String,
    name: String,
    unit: String,
}

fn run(bus: MasterBus) -> std::io::Result<()> {
    let devices = bus.devices_all();
    let mut meta: HashMap<(u32, i32), FieldMeta> = HashMap::new();
    let mut subs = Vec::new();

    for dev in &devices {
        let name = dev.name().unwrap_or_default();
        let class = name.split_whitespace().next().unwrap_or("").to_string();
        // Instance id = the device name without its leading class word (already
        // implied by the SK path category), path-sanitized; fall back to the name
        // then the numeric id.
        let label = name.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
        let instance = if !label.is_empty() {
            sanitize(&label)
        } else if !name.is_empty() {
            sanitize(&name)
        } else {
            dev.id().to_string()
        };

        let Ok(groups) = dev.tab(Menu::Monitoring) else { continue };
        let mut fields = Vec::new();
        for group in groups {
            for field in group.fields().unwrap_or_default() {
                let fname = field.name().unwrap_or_default();
                let unit = field.unit().unwrap_or_default();
                meta.insert(
                    (dev.id(), field.index()),
                    FieldMeta { class: class.clone(), instance: instance.clone(), name: fname, unit },
                );
                fields.push(field.index());
            }
        }
        if !fields.is_empty() {
            subs.push(bus.subscribe(dev.id(), fields, RATE, false));
        }
    }

    eprintln!(
        "masterbus-signalk: streaming {} fields from {} device(s)",
        meta.len(),
        devices.len()
    );

    let stdout = std::io::stdout();
    loop {
        let mut out = stdout.lock();
        let mut wrote = false;
        for sub in &subs {
            // Coalesce to the latest value per path this cycle: a field can be
            // updated many times between polls (the boat's real masters poll some
            // values rapidly, and we emit those too).
            let mut latest: HashMap<String, serde_json::Value> = HashMap::new();
            while let Some(u) = sub.try_recv() {
                if let Some(m) = meta.get(&(u.device, u.field))
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
                writeln!(out, "{}", serde_json::to_string(&delta).unwrap())?;
                wrote = true;
            }
        }
        if wrote {
            out.flush()?;
        }
        drop(out);
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

/// Keep only Signal K path-segment-safe characters in an instance id.
fn sanitize(s: &str) -> String {
    let cleaned: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
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
