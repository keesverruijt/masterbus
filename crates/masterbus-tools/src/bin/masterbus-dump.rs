//! Dump the whole bus — every device, group, field and (optionally) live value —
//! as a single JSON document.
//!
//! This is the "send me your bus" tool. A user with hardware runs it and attaches
//! the output to an issue; a developer without the hardware then has everything
//! needed to write a Signal K mapping for a device class: stable field ids, group
//! ids, names, units, ranges, enum option labels and typical values.
//!
//! ```text
//! masterbus-dump [options] [output.json]
//!
//!   -o, --output <file>   write here instead of stdout
//!       --menus <list>    monitoring,configuration,service,alarm,history | all
//!                         (default: monitoring,configuration,service)
//!       --values <mode>   none | monitoring | all   (default: monitoring)
//!       --device <hex>    only this device address (repeatable, e.g. 286CA9)
//!       --probe           also flat-probe the field-index space per device
//!                         (slow; finds fields no menu lists)
//!       --compact         one-line JSON instead of pretty-printed
//!   -h, --help            this text
//! ```
//!
//! Transport (USB / SocketCAN), master role, and the schema cache directory all
//! come from the per-host config file (see [`masterbus::FileConfig`]); the file
//! is created on first run.
//!
//! The host's Signal K mapping (`mapping.json`, see `masterbus_tools::mapping`)
//! is included when there is one, both as a whole and as a `signalk` path
//! beside each mapped field. A dump then shows the bus *and* the decisions
//! someone made about it, which is what makes a report useful for improving
//! the bundled suggestions.
//!
//! # Why the ids matter
//!
//! Device names, group names and field names are all installer-editable strings
//! held in the device's EEPROM — a charger's "Output 1" is routinely renamed to
//! "Eng.batt". The numbers are not: `field.id` (channel + wire index) and
//! `group.id` are fixed by the firmware. Mapping tables should key on those and
//! treat the names as documentation.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use masterbus::{
    AccessLevel, Channel, Config, DeviceId, DeviceStatus, FieldId, FieldInfo, MasterBus, Menu,
    Value, VisualizationType, field_id,
};
use masterbus_tools::mapping::Mapping;
use serde::Serialize;

/// Version of the JSON document shape, so a consumer can tell dumps apart.
const FORMAT: u32 = 1;

/// Menus dumped when `--menus` is not given.
const DEFAULT_MENUS: &[Menu] = &[Menu::Monitoring, Menu::Configuration, Menu::Service];

/// Which menus' fields get their live value read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueMode {
    /// Read nothing — schema only.
    None,
    /// Read the monitoring menu only (the default; keeps bus load sane).
    Monitoring,
    /// Read every dumped field.
    All,
}

impl ValueMode {
    fn name(self) -> &'static str {
        match self {
            ValueMode::None => "none",
            ValueMode::Monitoring => "monitoring",
            ValueMode::All => "all",
        }
    }

    fn wants(self, menu: Menu) -> bool {
        match self {
            ValueMode::None => false,
            ValueMode::Monitoring => menu == Menu::Monitoring,
            ValueMode::All => true,
        }
    }
}

/// Parsed command line.
struct Args {
    output: Option<PathBuf>,
    menus: Vec<Menu>,
    values: ValueMode,
    devices: Vec<DeviceId>,
    probe: bool,
    pretty: bool,
}

const USAGE: &str = "\
Usage: masterbus-dump [options] [output.json]

  -o, --output <file>   write here instead of stdout
      --menus <list>    monitoring,configuration,service,alarm,history | all
                        (default: monitoring,configuration,service)
      --values <mode>   none | monitoring | all   (default: monitoring)
      --device <hex>    only this device address (repeatable, e.g. 286CA9)
      --probe           also flat-probe the field-index space per device (slow)
      --compact         one-line JSON instead of pretty-printed
  -h, --help            show this help
";

fn parse_menu(s: &str) -> Option<Menu> {
    match s.trim().to_ascii_lowercase().as_str() {
        "monitoring" | "mon" => Some(Menu::Monitoring),
        "configuration" | "config" | "cfg" => Some(Menu::Configuration),
        "service" | "svc" => Some(Menu::Service),
        "alarm" | "alarms" => Some(Menu::Alarm),
        "history" | "hist" => Some(Menu::History),
        _ => None,
    }
}

/// Lowercase tag for a menu, used as the JSON `menu` value.
fn menu_tag(m: Menu) -> String {
    match m {
        Menu::Monitoring => "monitoring".into(),
        Menu::Configuration => "configuration".into(),
        Menu::Service => "service".into(),
        Menu::Alarm => "alarm".into(),
        Menu::History => "history".into(),
        Menu::Other(s) => format!("other({s})"),
    }
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// The parser proper, over an explicit argument list so it can be exercised
/// without touching the process environment.
fn parse_args_from(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        output: None,
        menus: DEFAULT_MENUS.to_vec(),
        values: ValueMode::Monitoring,
        devices: Vec::new(),
        probe: false,
        pretty: true,
    };
    let mut menus_set = false;
    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-o" | "--output" => {
                let v = it.next().ok_or("--output needs a file name")?;
                args.output = Some(PathBuf::from(v));
            }
            "--menus" => {
                let v = it.next().ok_or("--menus needs a value")?;
                let mut menus = Vec::new();
                if v.trim().eq_ignore_ascii_case("all") {
                    menus.extend_from_slice(&[
                        Menu::Monitoring,
                        Menu::Configuration,
                        Menu::Service,
                        Menu::Alarm,
                        Menu::History,
                    ]);
                } else {
                    for part in v.split(',').filter(|p| !p.trim().is_empty()) {
                        menus.push(parse_menu(part).ok_or(format!("unknown menu {part:?}"))?);
                    }
                }
                if menus.is_empty() {
                    return Err("--menus selected nothing".into());
                }
                args.menus = menus;
                menus_set = true;
            }
            "--values" => {
                let v = it.next().ok_or("--values needs a value")?;
                args.values = match v.trim().to_ascii_lowercase().as_str() {
                    "none" | "no" => ValueMode::None,
                    "monitoring" | "mon" => ValueMode::Monitoring,
                    "all" => ValueMode::All,
                    other => return Err(format!("unknown --values mode {other:?}")),
                };
            }
            "--device" => {
                let v = it.next().ok_or("--device needs an address")?;
                let t = v.trim().trim_start_matches("0x").trim_start_matches("0X");
                let id = u32::from_str_radix(t, 16)
                    .map_err(|_| format!("--device {v:?} is not a hex address"))?;
                args.devices.push(id);
            }
            "--probe" => args.probe = true,
            "--compact" => args.pretty = false,
            other if other.starts_with('-') => return Err(format!("unknown option {other:?}")),
            positional => {
                if args.output.is_some() {
                    return Err("output file given twice".into());
                }
                args.output = Some(PathBuf::from(positional));
            }
        }
    }
    // Reading values from a menu that is not being dumped can't happen, but
    // asking for monitoring values while not dumping monitoring is a user slip.
    if menus_set && args.values == ValueMode::Monitoring && !args.menus.contains(&Menu::Monitoring)
    {
        eprintln!(
            "masterbus-dump: note: --values monitoring but the monitoring menu is not in --menus; no values will be read"
        );
    }
    Ok(args)
}

/// The whole document.
#[derive(Serialize)]
struct Dump {
    /// Shape version of this document (see [`FORMAT`]).
    format: u32,
    /// Producing tool and version.
    tool: String,
    /// Wall-clock time of the dump, `YYYY-MM-DDTHH:MM:SSZ`.
    generated: String,
    /// Seconds since the Unix epoch, for consumers that want a number.
    generated_unix: u64,
    /// Menus that were enumerated.
    menus: Vec<String>,
    /// Which fields had their value read.
    values: String,
    /// Whether the flat field-index probe ran.
    probed: bool,
    /// The host's Signal K mapping, when there is one.
    ///
    /// A dump then carries both what the bus looks like and what a human
    /// decided it means, which is what lets the bundled suggestion database
    /// improve from a report. Omitted when no mapping file exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    mapping: Option<Mapping>,
    /// Where that mapping was read from.
    #[serde(skip_serializing_if = "Option::is_none")]
    mapping_path: Option<String>,
    /// Every device, in bus-address order.
    devices: Vec<DeviceDump>,
}

/// One device.
#[derive(Serialize)]
struct DeviceDump {
    /// Bus address as `0xRRGGBB` text (matches the TUI header).
    id: String,
    /// Bus address as a number.
    address: DeviceId,
    /// Article (model) number — the stable key for "what kind of device is this".
    article: String,
    /// Serial number — the stable key for "which unit is this".
    serial: String,
    /// Hardware revision code.
    revision: String,
    /// Installer-assigned device name. Not stable; do not key on it.
    name: String,
    /// Firmware version.
    firmware: String,
    /// Liveness-derived status at dump time.
    status: DeviceStatus,
    /// Access level the device reports, when it could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    access_level: Option<AccessLevel>,
    /// Anything that went wrong for this device, rather than aborting the dump.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<String>,
    /// Groups, in menu-then-discovery order.
    groups: Vec<GroupDump>,
    /// Fields the flat probe found that no dumped group lists (`--probe` only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ungrouped_fields: Vec<FieldDump>,
}

/// One group within a menu.
#[derive(Serialize)]
struct GroupDump {
    /// Global group id — stable per firmware.
    id: i32,
    /// Group name. Installer-editable on some devices; do not key on it.
    name: String,
    /// Which menu the group belongs to.
    menu: String,
    /// Fields in display order.
    fields: Vec<FieldDump>,
}

/// One field, with its metadata and optionally its value.
#[derive(Serialize)]
struct FieldDump {
    /// Channel-aware field id as `0x000`..`0x1FF` — the same text the TUI shows
    /// and `masterbus-set-field` takes. **This is the stable key for mapping.**
    id: String,
    /// The same id as a number.
    index: FieldId,
    /// Which wire channel the id lives on.
    channel: Channel,
    /// Field name. Installer-editable; documentation, not a key.
    name: String,
    /// Unit as the device reports it (may be empty).
    unit: String,
    /// Presentation / edit widget.
    viz_type: VisualizationType,
    /// Writable at the current access level.
    writeable: bool,
    /// Can be driven by an event on another device.
    eventable: bool,
    /// Minimum (numeric fields).
    min: f64,
    /// Maximum, or option count for lists.
    max: f64,
    /// Step (numeric fields).
    step: f64,
    /// Option labels (list/enum fields).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    options: Vec<String>,
    /// Decoded value, when read. Non-finite floats serialize as `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    /// The same value rendered the way the TUI shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    value_text: Option<String>,
    /// Why the value could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    value_error: Option<String>,
    /// The Signal K path this field publishes to, from the host's mapping.
    /// Put beside the field rather than only in the `mapping` block so a
    /// reader can see the decision next to the evidence for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    signalk: Option<String>,
}

/// Render a value the way `masterbus-tui` does, so a dump reads like the screen.
fn format_value(v: &Value) -> String {
    match v {
        Value::Float(x) if x.is_nan() => "—".into(),
        Value::Float(x) => format!("{x:.2}"),
        Value::Boolean(b) => if *b { "on" } else { "off" }.into(),
        Value::Date(d) if d.year < 0 || d.mon < 0 || d.day < 0 => "—".into(),
        Value::Date(d) => format!("{:04}-{:02}-{:02}", d.year, d.mon, d.day),
        Value::Time(t) if t.sec < 0 => "—".into(),
        Value::Time(t) => format!("{}d {:02}:{:02}:{:02}", t.days, t.hour, t.min, t.sec),
        Value::List { index, options } => options
            .get(*index as usize)
            .cloned()
            .unwrap_or_else(|| format!("[{index}]")),
        Value::Text { text, .. } => text.clone(),
        Value::DeviceRef { index, device_ids } => device_ids
            .get(*index as usize)
            .map(|d| format!("0x{d:06X}"))
            .unwrap_or_else(|| format!("[{index}]")),
        Value::Eventable { index, labels } => labels
            .get(*index as usize)
            .cloned()
            .unwrap_or_else(|| format!("[{index}]")),
        Value::Invalid => "invalid".into(),
    }
}

/// Build a [`FieldDump`] from schema metadata, without a value.
fn field_dump(f: &FieldInfo) -> FieldDump {
    FieldDump {
        id: format!("0x{:03X}", f.index),
        index: f.index,
        channel: field_id::channel(f.index),
        name: f.name.clone(),
        unit: f.unit.clone(),
        viz_type: f.viz_type,
        writeable: f.writeable,
        eventable: f.eventable,
        min: f.min,
        max: f.max,
        step: f.step,
        options: f.options.clone(),
        value: None,
        value_text: None,
        value_error: None,
        signalk: None,
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` from a Unix timestamp (proleptic Gregorian, UTC).
fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("masterbus-dump: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let bus = match MasterBus::auto(Config::default()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("masterbus-dump: connect failed: {e}");
            std::process::exit(2);
        }
    };

    let dump = collect(&bus, &args);

    let json = if args.pretty {
        serde_json::to_string_pretty(&dump)
    } else {
        serde_json::to_string(&dump)
    };
    let json = match json {
        Ok(j) => j,
        Err(e) => {
            eprintln!("masterbus-dump: could not serialize: {e}");
            std::process::exit(1);
        }
    };

    match &args.output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, format!("{json}\n")) {
                eprintln!("masterbus-dump: could not write {}: {e}", path.display());
                std::process::exit(1);
            }
            eprintln!(
                "masterbus-dump: wrote {} device(s) to {}",
                dump.devices.len(),
                path.display()
            );
        }
        None => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{json}");
        }
    }
}

/// Walk the bus and build the document. Per-device failures are recorded in
/// `errors` rather than aborting, so a partly-broken bus still yields a dump.
fn collect(bus: &MasterBus, args: &Args) -> Dump {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // The host's curated mapping, if any. A dump that carries it shows both the
    // bus and the decisions made about it, which is what a bug report needs.
    let mapping_path = std::env::var_os("MAPPING").map(PathBuf::from).or_else(|| {
        masterbus::FileConfig::load_or_create()
            .ok()
            .map(|c| c.mapping_path())
    });
    let mapping = mapping_path
        .as_deref()
        .and_then(|p| Mapping::load(p).ok())
        .filter(|m| !m.is_empty());

    let mut devices: Vec<_> = bus.devices_all();
    devices.sort_by_key(|d| d.id());

    let mut out = Vec::new();
    for dev in &devices {
        if !args.devices.is_empty() && !args.devices.contains(&dev.id()) {
            continue;
        }
        let mut errors = Vec::new();
        let identity = match dev.identity() {
            Ok(i) => i,
            Err(e) => {
                errors.push(format!("identity: {e}"));
                masterbus::DeviceIdentity {
                    article: String::new(),
                    serial: String::new(),
                    revision: String::new(),
                    name: String::new(),
                    firmware: String::new(),
                }
            }
        };
        eprintln!(
            "masterbus-dump: 0x{:06X} {} …",
            dev.id(),
            if identity.name.is_empty() {
                "(unnamed)"
            } else {
                &identity.name
            }
        );

        let mut groups = Vec::new();
        let mut seen: HashSet<FieldId> = HashSet::new();
        for &menu in &args.menus {
            let infos = match dev.tab_info(menu) {
                Ok(g) => g,
                Err(e) => {
                    errors.push(format!("{} menu: {e}", menu_tag(menu)));
                    continue;
                }
            };
            for g in infos {
                let mut fields = Vec::new();
                for f in &g.fields {
                    seen.insert(f.index);
                    let mut fd = field_dump(f);
                    fd.signalk = mapping
                        .as_ref()
                        .and_then(|m| m.field(&identity.serial, f.index))
                        .map(|fm| fm.path.clone());
                    if args.values.wants(menu) {
                        match dev.field(f.index).value() {
                            Ok(v) => {
                                fd.value_text = Some(format_value(&v));
                                fd.value = Some(v);
                            }
                            Err(e) => fd.value_error = Some(e.to_string()),
                        }
                    }
                    fields.push(fd);
                }
                groups.push(GroupDump {
                    id: g.id,
                    name: g.name.clone(),
                    menu: menu_tag(menu),
                    fields,
                });
            }
        }

        let mut ungrouped = Vec::new();
        if args.probe {
            match dev.all_fields() {
                Ok(all) => {
                    for f in &all {
                        if seen.insert(f.index) {
                            let mut fd = field_dump(f);
                            fd.signalk = mapping
                                .as_ref()
                                .and_then(|m| m.field(&identity.serial, f.index))
                                .map(|fm| fm.path.clone());
                            if args.values == ValueMode::All {
                                match dev.field(f.index).value() {
                                    Ok(v) => {
                                        fd.value_text = Some(format_value(&v));
                                        fd.value = Some(v);
                                    }
                                    Err(e) => fd.value_error = Some(e.to_string()),
                                }
                            }
                            ungrouped.push(fd);
                        }
                    }
                }
                Err(e) => errors.push(format!("field probe: {e}")),
            }
        }

        out.push(DeviceDump {
            id: format!("0x{:06X}", dev.id()),
            address: dev.id(),
            article: identity.article,
            serial: identity.serial,
            revision: identity.revision,
            name: identity.name,
            firmware: identity.firmware,
            status: dev.status(),
            access_level: dev.cached_access_level(),
            errors,
            groups,
            ungrouped_fields: ungrouped,
        });
    }

    Dump {
        format: FORMAT,
        tool: format!("masterbus-dump {}", env!("CARGO_PKG_VERSION")),
        generated: iso8601_utc(now),
        generated_unix: now,
        menus: args.menus.iter().copied().map(menu_tag).collect(),
        values: args.values.name().to_string(),
        probed: args.probe,
        mapping_path: mapping
            .is_some()
            .then(|| mapping_path.as_ref().map(|p| p.display().to_string()))
            .flatten(),
        mapping,
        devices: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day, to exercise the civil-from-days arithmetic.
        assert_eq!(iso8601_utc(1_709_208_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn field_id_text_matches_the_tui_and_cli_encoding() {
        let f = FieldInfo {
            index: field_id::btm3(0x0C),
            name: "Charge current".into(),
            unit: "A".into(),
            viz_type: VisualizationType::Float,
            writeable: false,
            eventable: false,
            min: 0.0,
            max: 100.0,
            step: 0.1,
            options: vec![],
        };
        let d = field_dump(&f);
        assert_eq!(d.id, "0x10C");
        assert_eq!(d.channel, Channel::Btm3);
    }

    #[test]
    fn nonfinite_floats_serialize_as_null_rather_than_failing() {
        let v = Value::Float(f32::NAN);
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"Float":null}"#);
    }

    /// Pins the document shape a consumer (or an AI reading an attached dump)
    /// relies on: hex `id` next to numeric `index`, value plus rendered text,
    /// and no `null` noise for the fields that were not read.
    #[test]
    fn document_shape_is_stable() {
        let f = FieldInfo {
            index: field_id::btm1(0x06),
            name: "Output 3".into(),
            unit: "V".into(),
            viz_type: VisualizationType::Float,
            writeable: false,
            eventable: false,
            min: 0.0,
            max: 32.0,
            step: 0.01,
            options: vec![],
        };
        let mut fd = field_dump(&f);
        let v = Value::Float(13.29);
        fd.value_text = Some(format_value(&v));
        fd.value = Some(v);
        let dump = Dump {
            format: FORMAT,
            tool: "masterbus-dump test".into(),
            generated: iso8601_utc(0),
            generated_unix: 0,
            menus: vec![menu_tag(Menu::Monitoring)],
            values: ValueMode::Monitoring.name().into(),
            probed: false,
            mapping: None,
            mapping_path: None,
            devices: vec![DeviceDump {
                id: "0x286CA9".into(),
                address: 0x286CA9,
                article: "40200500".into(),
                serial: "1234567".into(),
                revision: "A".into(),
                name: "CHG 12V ChargerE".into(),
                firmware: "1.9".into(),
                status: DeviceStatus::On,
                access_level: None,
                errors: vec![],
                groups: vec![GroupDump {
                    id: 2,
                    name: "Output".into(),
                    menu: menu_tag(Menu::Monitoring),
                    fields: vec![fd],
                }],
                ungrouped_fields: vec![],
            }],
        };
        let j: serde_json::Value = serde_json::from_str(&serde_json::to_string(&dump).unwrap())
            .expect("dump must be valid JSON");
        let field = &j["devices"][0]["groups"][0]["fields"][0];
        assert_eq!(field["id"], "0x006");
        assert_eq!(field["index"], 6);
        assert_eq!(field["channel"], "Btm1");
        assert_eq!(field["unit"], "V");
        assert_eq!(field["value"]["Float"], 13.29);
        assert_eq!(field["value_text"], "13.29");
        // Absent rather than null, so a dump stays readable.
        assert!(field.get("value_error").is_none());
        assert!(field.get("options").is_none());
        assert!(field.get("signalk").is_none());
        assert!(j["devices"][0].get("access_level").is_none());
        assert_eq!(j["devices"][0]["groups"][0]["menu"], "monitoring");
    }

    /// The mapping block and the per-field path are both optional, and a dump
    /// from a host with no mapping must not sprout empty keys.
    #[test]
    fn the_mapping_is_absent_rather_than_null_when_there_is_none() {
        let dump = Dump {
            format: FORMAT,
            tool: "masterbus-dump test".into(),
            generated: iso8601_utc(0),
            generated_unix: 0,
            menus: vec![menu_tag(Menu::Monitoring)],
            values: ValueMode::None.name().into(),
            probed: false,
            mapping: None,
            mapping_path: None,
            devices: vec![],
        };
        let j: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&dump).unwrap()).unwrap();
        assert!(j.get("mapping").is_none());
        assert!(j.get("mapping_path").is_none());
    }

    #[test]
    fn a_mapped_field_carries_its_signalk_path() {
        let f = FieldInfo {
            index: field_id::btm1(0x00E),
            name: "Battery voltage".into(),
            unit: "V".into(),
            viz_type: VisualizationType::Float,
            writeable: false,
            eventable: false,
            min: 0.0,
            max: 32.0,
            step: 0.01,
            options: vec![],
        };
        let mut fd = field_dump(&f);
        assert!(fd.signalk.is_none());
        fd.signalk = Some("electrical.chargers.ch1.voltage".into());
        let j: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&fd).unwrap()).unwrap();
        assert_eq!(j["signalk"], "electrical.chargers.ch1.voltage");
    }

    #[test]
    fn menu_tags_round_trip_through_the_parser() {
        for m in [
            Menu::Monitoring,
            Menu::Configuration,
            Menu::Service,
            Menu::Alarm,
            Menu::History,
        ] {
            assert_eq!(parse_menu(&menu_tag(m)), Some(m));
        }
    }
}

#[cfg(test)]
mod arg_tests {
    use super::*;

    fn parse(argv: &[&str]) -> Result<Args, String> {
        parse_args_from(argv.iter().map(|s| s.to_string()))
    }

    /// The message a rejected command line produces.
    fn err(argv: &[&str]) -> String {
        match parse(argv) {
            Err(e) => e,
            Ok(_) => panic!("expected {argv:?} to be rejected"),
        }
    }

    /// With no arguments: the three Btm1 menus, monitoring values only, and
    /// pretty JSON on stdout. Keeping the default bus load sane is the point
    /// of `--values monitoring`.
    #[test]
    fn the_defaults_are_the_three_btm1_menus_and_monitoring_values() {
        let a = parse(&[]).unwrap();
        assert!(a.output.is_none());
        assert_eq!(a.menus, DEFAULT_MENUS.to_vec());
        assert!(a.values == ValueMode::Monitoring);
        assert!(a.devices.is_empty());
        assert!(!a.probe);
        assert!(a.pretty);
    }

    /// The output file can be a flag or a positional, but not both and not
    /// twice — silently overwriting the wrong file would be worse than an
    /// error.
    #[test]
    fn the_output_file_may_be_a_flag_or_a_positional_but_not_two() {
        for argv in [
            vec!["-o", "dump.json"],
            vec!["--output", "dump.json"],
            vec!["dump.json"],
        ] {
            let a = parse(&argv).unwrap();
            assert_eq!(a.output, Some(PathBuf::from("dump.json")), "{argv:?}");
        }

        assert!(parse(&["a.json", "b.json"]).is_err());
        assert!(parse(&["-o", "a.json", "b.json"]).is_err());
        assert_eq!(err(&["-o"]), "--output needs a file name");
    }

    #[test]
    fn menus_accept_a_list_of_aliases_or_all() {
        let a = parse(&["--menus", "mon,cfg,svc"]).unwrap();
        assert_eq!(
            a.menus,
            vec![Menu::Monitoring, Menu::Configuration, Menu::Service]
        );

        let a = parse(&["--menus", "all"]).unwrap();
        assert_eq!(a.menus.len(), 5);
        assert!(a.menus.contains(&Menu::Alarm));
        assert!(a.menus.contains(&Menu::History));

        // Case and stray whitespace don't matter; empty entries are skipped.
        let a = parse(&["--menus", " Alarm , ,HIST "]).unwrap();
        assert_eq!(a.menus, vec![Menu::Alarm, Menu::History]);
    }

    /// A menu list that selects nothing is a mistake, not an empty dump.
    #[test]
    fn an_empty_or_unknown_menu_list_is_rejected() {
        assert_eq!(err(&["--menus", " , "]), "--menus selected nothing");
        assert_eq!(
            err(&["--menus", "monitoring,bogus"]),
            "unknown menu \"bogus\""
        );
        assert_eq!(err(&["--menus"]), "--menus needs a value");
    }

    #[test]
    fn every_value_mode_is_accepted_by_name() {
        for (arg, want) in [
            ("none", ValueMode::None),
            ("no", ValueMode::None),
            ("monitoring", ValueMode::Monitoring),
            ("mon", ValueMode::Monitoring),
            ("all", ValueMode::All),
            ("ALL", ValueMode::All),
        ] {
            let a = parse(&["--values", arg]).unwrap();
            assert!(a.values == want, "{arg}");
        }
        assert_eq!(err(&["--values", "some"]), "unknown --values mode \"some\"");
        assert_eq!(err(&["--values"]), "--values needs a value");
    }

    /// `--device` is repeatable and takes the same hex form the TUI shows,
    /// with or without the `0x` prefix.
    #[test]
    fn devices_are_hex_and_repeatable() {
        let a = parse(&["--device", "188EA2", "--device", "0x3A3B4B"]).unwrap();
        assert_eq!(a.devices, vec![0x188EA2, 0x3A3B4B]);

        assert!(parse(&["--device", "zzz"]).is_err());
        assert_eq!(err(&["--device"]), "--device needs an address");
    }

    #[test]
    fn the_boolean_flags_flip_their_defaults() {
        let a = parse(&["--probe", "--compact"]).unwrap();
        assert!(a.probe);
        assert!(!a.pretty);
    }

    /// An unrecognised option is an error; a bare word is the output file.
    #[test]
    fn an_unknown_option_is_rejected() {
        assert_eq!(err(&["--nope"]), "unknown option \"--nope\"");
        assert!(parse(&["-x"]).is_err());
    }

    #[test]
    fn menus_parse_from_every_documented_alias() {
        for (s, want) in [
            ("monitoring", Menu::Monitoring),
            ("mon", Menu::Monitoring),
            ("configuration", Menu::Configuration),
            ("config", Menu::Configuration),
            ("cfg", Menu::Configuration),
            ("service", Menu::Service),
            ("svc", Menu::Service),
            ("alarm", Menu::Alarm),
            ("alarms", Menu::Alarm),
            ("history", Menu::History),
            ("hist", Menu::History),
        ] {
            assert_eq!(parse_menu(s), Some(want), "{s}");
        }
        assert_eq!(parse_menu("settings"), None);
    }

    /// The JSON `menu` tag is the stable name a consumer keys on, including
    /// for a selector this build has no name for.
    #[test]
    fn every_menu_has_a_json_tag() {
        assert_eq!(menu_tag(Menu::Monitoring), "monitoring");
        assert_eq!(menu_tag(Menu::Configuration), "configuration");
        assert_eq!(menu_tag(Menu::Service), "service");
        assert_eq!(menu_tag(Menu::Alarm), "alarm");
        assert_eq!(menu_tag(Menu::History), "history");
        assert_eq!(menu_tag(Menu::Other(0x07)), "other(7)");
    }

    /// Which menus get their values read. The default reads only monitoring,
    /// so a dump of every menu doesn't hammer the bus.
    #[test]
    fn the_value_mode_decides_which_menus_are_read() {
        assert!(!ValueMode::None.wants(Menu::Monitoring));
        assert!(ValueMode::Monitoring.wants(Menu::Monitoring));
        assert!(!ValueMode::Monitoring.wants(Menu::Configuration));
        assert!(ValueMode::All.wants(Menu::Monitoring));
        assert!(ValueMode::All.wants(Menu::History));

        assert_eq!(ValueMode::None.name(), "none");
        assert_eq!(ValueMode::Monitoring.name(), "monitoring");
        assert_eq!(ValueMode::All.name(), "all");
    }

    /// The dump renders values the way the TUI does, so a dump reads like the
    /// screen — except a device reference, which keeps its hex id because a
    /// file has no device list to resolve against.
    #[test]
    fn values_render_the_way_the_screen_shows_them() {
        assert_eq!(format_value(&Value::Float(26.3456)), "26.35");
        assert_eq!(format_value(&Value::Float(f32::NAN)), "—");
        assert_eq!(format_value(&Value::Boolean(true)), "on");
        assert_eq!(format_value(&Value::Boolean(false)), "off");
        assert_eq!(format_value(&Value::Invalid), "invalid");
        assert_eq!(
            format_value(&Value::Text {
                sid: 1,
                text: "Nav Chg".into()
            }),
            "Nav Chg"
        );
        assert_eq!(
            format_value(&Value::List {
                index: 1,
                options: vec!["Off".into(), "On".into()]
            }),
            "On"
        );
        assert_eq!(
            format_value(&Value::Eventable {
                index: 4,
                labels: vec![]
            }),
            "[4]"
        );
        assert_eq!(
            format_value(&Value::DeviceRef {
                index: 0,
                device_ids: vec![0x188EA2]
            }),
            "0x188EA2"
        );
        assert_eq!(
            format_value(&Value::DeviceRef {
                index: 3,
                device_ids: vec![]
            }),
            "[3]"
        );
    }

    #[test]
    fn date_and_time_sentinels_render_as_a_dash() {
        use masterbus::{Date, Time};
        assert_eq!(
            format_value(&Value::Date(Date {
                day: -1,
                mon: -1,
                year: -1
            })),
            "—"
        );
        assert_eq!(
            format_value(&Value::Date(Date {
                day: 7,
                mon: 5,
                year: 2026
            })),
            "2026-05-07"
        );
        assert_eq!(
            format_value(&Value::Time(Time {
                sec: -1,
                min: 0,
                hour: 0,
                days: 0
            })),
            "—"
        );
        assert_eq!(
            format_value(&Value::Time(Time {
                sec: 5,
                min: 4,
                hour: 3,
                days: 2
            })),
            "2d 03:04:05"
        );
    }
}
