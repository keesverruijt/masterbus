//! Read every device's events and watch how their targets are stored.
//!
//! An event is four Configuration fields: a source, a target device, a
//! command (which of the target's outputs) and an action. The target reads
//! back as a small number, which matched the target's place in the bus
//! device list sorted by address (PROTOCOL.md §9a). Whether the device keeps
//! that number or the target's MasterBus id is not known. If it keeps the
//! id, the number it reports changes when a device below the target leaves
//! the bus, and the event keeps driving the same device. This tool shows
//! that, and **never writes anything**.
//!
//! ```text
//! masterbus-events [options]
//!
//!   -o, --output <file>   also write everything as JSON (rewritten after
//!                         every round, so stopping with Ctrl-C keeps it)
//!       --device <hex>    only this device's events (repeatable)
//!       --watch <secs>    keep reading every <secs> seconds and print what
//!                         changes, until Ctrl-C (default: read once)
//!   -V, --version         print the version
//!   -h, --help            this text
//! ```
//!
//! Each round reads, live, every event's target and command with the
//! maximum the device reports for them, and the devices on the bus. With
//! `--watch`, switch one device off and on again and see what the numbers
//! do. The bus is shared with masterbus-signalk when it runs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use masterbus::protocol::meta_op;
use masterbus::{
    Config, Device, DeviceId, FieldId, FieldInfo, MasterBus, Menu, Value, VisualizationType,
};
use serde::Serialize;

const USAGE: &str = "usage: masterbus-events [-o file.json] [--device HEX]... [--watch SECS]";

struct Args {
    output: Option<PathBuf>,
    devices: Vec<DeviceId>,
    watch: Option<Duration>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        output: None,
        devices: Vec::new(),
        watch: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |what: &str| it.next().ok_or_else(|| format!("{a} needs {what}"));
        match a.as_str() {
            "-o" | "--output" => args.output = Some(PathBuf::from(value("a file")?)),
            "--device" => {
                let v = value("a device address")?;
                let hex = v.trim_start_matches("0x");
                let id = DeviceId::from_str_radix(hex, 16)
                    .map_err(|_| format!("--device {v}: not a hex address"))?;
                args.devices.push(id);
            }
            "--watch" => {
                let v = value("seconds")?;
                let secs: u64 = v
                    .parse()
                    .ok()
                    .filter(|&s| s > 0)
                    .ok_or_else(|| format!("--watch {v}: not a number of seconds"))?;
                args.watch = Some(Duration::from_secs(secs));
            }
            "-V" | "--version" => {
                println!("masterbus-events {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(args)
}

/// One event's four fields, found by their shape: a `DeviceList` field
/// directly followed by an `EventCommand` is a target and its command; the
/// field before is the source and the one after the action.
#[derive(Clone)]
struct Event {
    device: DeviceId,
    group: String,
    source: Option<FieldInfo>,
    target: FieldInfo,
    command: FieldInfo,
    action: Option<FieldInfo>,
}

/// A `DeviceList` field that is not an event target, such as an APR's
/// "Shunt device". Its list may be a filtered one (PROTOCOL.md §9a).
#[derive(Clone)]
struct DeviceRefField {
    device: DeviceId,
    group: String,
    field: FieldInfo,
}

fn find_events(
    device: DeviceId,
    groups: &[masterbus::GroupInfo],
) -> (Vec<Event>, Vec<DeviceRefField>) {
    let mut events = Vec::new();
    let mut refs = Vec::new();
    for g in groups {
        let f = &g.fields;
        for i in 0..f.len() {
            if f[i].viz_type != VisualizationType::DeviceList {
                continue;
            }
            match f.get(i + 1) {
                Some(cmd) if cmd.viz_type == VisualizationType::EventCommand => {
                    events.push(Event {
                        device,
                        group: g.name.clone(),
                        source: i.checked_sub(1).map(|p| f[p].clone()),
                        target: f[i].clone(),
                        command: cmd.clone(),
                        action: f.get(i + 2).cloned(),
                    })
                }
                _ => refs.push(DeviceRefField {
                    device,
                    group: g.name.clone(),
                    field: f[i].clone(),
                }),
            }
        }
    }
    (events, refs)
}

/// A number the device reports for a field now: its value and its maximum.
#[derive(Serialize, Clone, PartialEq)]
struct Reading {
    value: Option<i32>,
    max: Option<f32>,
}

fn read(dev: &Device, f: &FieldInfo) -> Reading {
    let value = dev.field(f.index).value().ok().and_then(|v| v.index());
    let max = dev
        .property(f.index, meta_op::MAX, None)
        .ok()
        .flatten()
        .filter(|r| r.len() >= 8)
        .map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]));
    Reading { value, max }
}

/// The label of a list field's current value.
fn label(dev: &Device, f: &FieldInfo) -> Option<String> {
    let v = dev.field(f.index).value().ok()?;
    let i = v.index()?;
    match v {
        Value::List { options, .. } if !options.is_empty() => options.get(i as usize).cloned(),
        _ => f.options.get(i as usize).cloned(),
    }
    .or(Some(format!("[{i}]")))
}

#[derive(Serialize)]
struct BusDevice {
    position: usize,
    address: String,
    name: String,
}

#[derive(Serialize)]
struct EventReport {
    device: String,
    device_name: String,
    group: String,
    source: Option<String>,
    target_field: String,
    target: Reading,
    /// The target as the bus device list sorted by address would have it.
    target_guess: Option<String>,
    command_field: String,
    command: Reading,
    /// The guessed target's output at that position.
    command_guess: Option<String>,
    action: Option<String>,
}

#[derive(Serialize)]
struct RefReport {
    device: String,
    device_name: String,
    group: String,
    field: String,
    name: String,
    reading: Reading,
}

#[derive(Serialize)]
struct Round {
    /// Seconds since the first round.
    at: f64,
    bus: Vec<BusDevice>,
    events: Vec<EventReport>,
    device_refs: Vec<RefReport>,
}

/// The option labels a target field answers for (op `0x26`), raw. A device
/// that keeps MasterBus ids might hand them out here.
#[derive(Serialize)]
struct OptionProbe {
    device: String,
    field: String,
    options: Vec<(u8, Option<String>)>,
}

#[derive(Serialize)]
struct Report {
    version: &'static str,
    started: u64,
    option_probes: Vec<OptionProbe>,
    rounds: Vec<Round>,
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

struct Names(HashMap<DeviceId, String>);

impl Names {
    fn get(&mut self, bus: &MasterBus, id: DeviceId) -> String {
        self.0
            .entry(id)
            .or_insert_with(|| {
                bus.device(id)
                    .name()
                    .ok()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("{id:06X}"))
            })
            .clone()
    }
}

fn round(
    bus: &MasterBus,
    names: &mut Names,
    events: &[Event],
    refs: &[DeviceRefField],
    t0: Instant,
) -> Round {
    let mut on_bus: Vec<DeviceId> = bus.devices().iter().map(|d| d.id()).collect();
    on_bus.sort_unstable();
    let bus_list = on_bus
        .iter()
        .enumerate()
        .map(|(position, &id)| BusDevice {
            position,
            address: format!("{id:06X}"),
            name: names.get(bus, id),
        })
        .collect();
    let mut outputs: HashMap<DeviceId, Vec<String>> = HashMap::new();
    let mut reports = Vec::new();
    for e in events {
        let dev = bus.device(e.device);
        let target = read(&dev, &e.target);
        let command = read(&dev, &e.command);
        let guessed = target
            .value
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| on_bus.get(i).copied());
        let command_guess = guessed.and_then(|t| {
            let outs = outputs.entry(t).or_insert_with(|| {
                let d = bus.device(t);
                d.eventable_output_fields()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f: FieldId| {
                        d.field_info(f)
                            .ok()
                            .flatten()
                            .map(|i| i.name)
                            .unwrap_or_else(|| format!("field 0x{f:03X}"))
                    })
                    .collect()
            });
            command
                .value
                .and_then(|k| usize::try_from(k).ok())
                .and_then(|k| outs.get(k).cloned())
        });
        reports.push(EventReport {
            device: format!("{:06X}", e.device),
            device_name: names.get(bus, e.device),
            group: e.group.clone(),
            source: e.source.as_ref().and_then(|f| label(&dev, f)),
            target_field: format!("0x{:03X} {}", e.target.index, e.target.name),
            target,
            target_guess: guessed.map(|t| format!("{} ({t:06X})", names.get(bus, t))),
            command_field: format!("0x{:03X} {}", e.command.index, e.command.name),
            command,
            command_guess,
            action: e.action.as_ref().and_then(|f| label(&dev, f)),
        });
    }
    let device_refs = refs
        .iter()
        .map(|r| RefReport {
            device: format!("{:06X}", r.device),
            device_name: names.get(bus, r.device),
            group: r.group.clone(),
            field: format!("0x{:03X}", r.field.index),
            name: r.field.name.clone(),
            reading: read(&bus.device(r.device), &r.field),
        })
        .collect();
    Round {
        at: t0.elapsed().as_secs_f64(),
        bus: bus_list,
        events: reports,
        device_refs,
    }
}

fn num(r: &Reading) -> String {
    let v = r.value.map_or("?".into(), |v| v.to_string());
    let m = r.max.map_or("?".into(), |m| format!("{m}"));
    format!("{v} (max {m})")
}

fn print_round(r: &Round) {
    println!("Devices on the bus, sorted by address:");
    for d in &r.bus {
        println!("  {:>2}  {}  {}", d.position, d.address, d.name);
    }
    println!();
    if r.events.is_empty() {
        println!("No events found.");
    }
    for e in &r.events {
        println!(
            "{} [{}] {}: {}",
            e.device_name,
            e.device,
            e.target_field.split_once(' ').map_or("", |(_, n)| n),
            e.source.as_deref().unwrap_or("?"),
        );
        println!(
            "    target  {:<16} → {}",
            num(&e.target),
            e.target_guess.as_deref().unwrap_or("?")
        );
        println!(
            "    command {:<16} → {}",
            num(&e.command),
            e.command_guess.as_deref().unwrap_or("?")
        );
        println!("    action  {}", e.action.as_deref().unwrap_or("?"));
    }
    if !r.device_refs.is_empty() {
        println!();
        println!("Other device references:");
        for d in &r.device_refs {
            println!(
                "  {} [{}] {} {}: {}",
                d.device_name,
                d.device,
                d.field,
                d.name,
                num(&d.reading)
            );
        }
    }
}

/// What changed between two rounds, one line each.
fn changes(old: &Round, new: &Round) -> Vec<String> {
    let mut out = Vec::new();
    let addrs = |r: &Round| -> Vec<String> { r.bus.iter().map(|d| d.address.clone()).collect() };
    let (was, is) = (addrs(old), addrs(new));
    for d in &new.bus {
        if !was.contains(&d.address) {
            out.push(format!("joined the bus: {} {}", d.address, d.name));
        }
    }
    for d in &old.bus {
        if !is.contains(&d.address) {
            out.push(format!("left the bus: {} {}", d.address, d.name));
        }
    }
    for (a, b) in old.events.iter().zip(&new.events) {
        let what = format!("{} [{}] {}", b.device_name, b.device, b.target_field);
        if a.target != b.target {
            out.push(format!(
                "{what}: target {} → {} (guess now: {})",
                num(&a.target),
                num(&b.target),
                b.target_guess.as_deref().unwrap_or("?")
            ));
        }
        if a.command != b.command {
            out.push(format!(
                "{what}: command {} → {}",
                num(&a.command),
                num(&b.command)
            ));
        }
    }
    for (a, b) in old.device_refs.iter().zip(&new.device_refs) {
        if a.reading != b.reading {
            out.push(format!(
                "{} [{}] {} {}: {} → {}",
                b.device_name,
                b.device,
                b.field,
                b.name,
                num(&a.reading),
                num(&b.reading)
            ));
        }
    }
    out
}

fn save(path: &Option<PathBuf>, report: &Report) {
    let Some(path) = path else { return };
    match serde_json::to_string_pretty(report) {
        Ok(j) => {
            if let Err(e) = std::fs::write(path, format!("{j}\n")) {
                eprintln!("masterbus-events: could not write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("masterbus-events: could not serialize: {e}"),
    }
}

fn clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let tod = secs % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
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
            eprintln!("masterbus-events: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    // Every value is read from the device, never from the cache.
    let config = Config {
        max_age: Duration::ZERO,
        ..Config::default()
    };
    let bus = match MasterBus::auto_joined(config) {
        Ok((b, relay)) => {
            if let Some(a) = relay {
                eprintln!("masterbus-events: using the bus shared by masterbus-signalk ({a})");
            }
            b
        }
        Err(e) => {
            eprintln!("masterbus-events: connect failed: {e}");
            std::process::exit(2);
        }
    };

    let mut all: Vec<DeviceId> = bus.devices_all().iter().map(|d| d.id()).collect();
    all.sort_unstable();
    let mut names = Names(HashMap::new());
    let mut events = Vec::new();
    let mut refs = Vec::new();
    for &id in &all {
        if !args.devices.is_empty() && !args.devices.contains(&id) {
            continue;
        }
        let name = names.get(&bus, id);
        eprintln!("masterbus-events: reading the configuration of {name} [{id:06X}] …");
        match bus.device(id).tab_info(Menu::Configuration) {
            Ok(groups) => {
                let (e, r) = find_events(id, &groups);
                events.extend(e);
                refs.extend(r);
            }
            Err(e) => eprintln!("masterbus-events: {name} [{id:06X}]: {e}"),
        }
    }

    // What each target field answers for its option labels: once, raw.
    let mut option_probes = Vec::new();
    for e in &events {
        let dev = bus.device(e.device);
        let max = read(&dev, &e.target).max.unwrap_or(0.0).clamp(0.0, 31.0) as u8;
        let options = (0..=max)
            .map(|k| {
                let r = dev.property(e.target.index, meta_op::OPTION, Some(k));
                // An answer of just the request's four bytes carries no
                // label: the device has none for that option.
                let answer = r.ok().flatten().map(|b| {
                    if b.len() <= 4 {
                        "no label".to_string()
                    } else {
                        hex(&b)
                    }
                });
                (k, answer)
            })
            .collect();
        option_probes.push(OptionProbe {
            device: format!("{:06X}", e.device),
            field: format!("0x{:03X} {}", e.target.index, e.target.name),
            options,
        });
    }

    let t0 = Instant::now();
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut report = Report {
        version: env!("CARGO_PKG_VERSION"),
        started,
        option_probes,
        rounds: Vec::new(),
    };
    let first = round(&bus, &mut names, &events, &refs, t0);
    println!();
    print_round(&first);
    println!();
    println!("Option labels the event targets answer (op 0x26, raw):");
    for p in &report.option_probes {
        let answered: Vec<String> = p
            .options
            .iter()
            .filter_map(|(k, r)| r.as_ref().map(|r| format!("{k}: {r}")))
            .collect();
        println!(
            "  [{}] {}: {}",
            p.device,
            p.field,
            if answered.is_empty() {
                "no answer".to_string()
            } else if p
                .options
                .iter()
                .all(|(_, r)| r.as_deref().is_none_or(|r| r == "no label"))
            {
                "no labels".to_string()
            } else {
                answered.join(", ")
            }
        );
    }
    report.rounds.push(first);
    save(&args.output, &report);

    let Some(every) = args.watch else { return };
    println!();
    println!(
        "Watching every {} s. Switch a device off and on again; Ctrl-C to stop.",
        every.as_secs()
    );
    loop {
        std::thread::sleep(every);
        let r = round(&bus, &mut names, &events, &refs, t0);
        let prev = report.rounds.last().expect("the first round");
        for line in changes(prev, &r) {
            println!("{}  {line}", clock());
        }
        report.rounds.push(r);
        save(&args.output, &report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masterbus::GroupInfo;

    fn field(index: FieldId, name: &str, viz: VisualizationType) -> FieldInfo {
        FieldInfo {
            index,
            name: name.into(),
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

    /// A target followed by a command is an event, with the fields around
    /// it as its source and action; any other device list is a reference.
    #[test]
    fn events_are_found_by_their_shape() {
        use VisualizationType as V;
        let groups = vec![GroupInfo {
            id: 1,
            name: "Events".into(),
            menu: Menu::Configuration,
            fields: vec![
                field(0x112, "Event 1 source", V::DropDown),
                field(0x113, "Event 1 target", V::DeviceList),
                field(0x114, "Event 1 command", V::EventCommand),
                field(0x115, "Event 1 data", V::DropDown),
                field(0x122, "Shunt device", V::DeviceList),
            ],
        }];
        let (events, refs) = find_events(0x43DF24, &groups);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.source.as_ref().unwrap().index, 0x112);
        assert_eq!((e.target.index, e.command.index), (0x113, 0x114));
        assert_eq!(e.action.as_ref().unwrap().index, 0x115);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].field.index, 0x122);
    }

    fn report(value: i32, max: f32) -> EventReport {
        EventReport {
            device: "43DF24".into(),
            device_name: "DSI".into(),
            group: "Events".into(),
            source: None,
            target_field: "0x113 Event 1 target".into(),
            target: Reading {
                value: Some(value),
                max: Some(max),
            },
            target_guess: None,
            command_field: "0x114 Event 1 command".into(),
            command: Reading {
                value: Some(0),
                max: Some(1.0),
            },
            command_guess: None,
            action: None,
        }
    }

    fn bus(addrs: &[&str]) -> Vec<BusDevice> {
        addrs
            .iter()
            .enumerate()
            .map(|(position, a)| BusDevice {
                position,
                address: a.to_string(),
                name: format!("dev {a}"),
            })
            .collect()
    }

    /// A device leaving and the target number moving with it are both
    /// reported; what stays the same is not.
    #[test]
    fn a_device_leaving_and_a_target_moving_are_reported() {
        let old = Round {
            at: 0.0,
            bus: bus(&["100000", "200000", "300000"]),
            events: vec![report(2, 2.0)],
            device_refs: vec![],
        };
        let new = Round {
            at: 5.0,
            bus: bus(&["200000", "300000"]),
            events: vec![report(1, 1.0)],
            device_refs: vec![],
        };
        let lines = changes(&old, &new);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("left the bus: 100000"));
        assert!(lines[1].contains("target 2 (max 2) → 1 (max 1)"));
        assert!(changes(&new, &new).is_empty());
    }
}
