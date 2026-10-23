//! Test tool for #29: does a device act on a value write whose commit names
//! a writer of our own, rather than the token captured from MasterAdjust?
//!
//! ```text
//! masterbus-event-test <device_id> <field_id>
//! ```
//!
//! Point it at an on/off output that is safe to switch (a spare channel). It
//! switches the output over and back with each way of writing in turn, reads
//! the output back after each write, and prints what happened. Every frame
//! it sees goes to `event-test.log` in the current folder.
//!
//! It also asks the device for its eventable outputs with `[0x08, 0x0D]` and
//! `[0x08, 0x40 + K]`, the queries seen in the #29 capture.
//!
//! It sends its own frames, so it uses the bus connection without an engine:
//! the one `masterbus-signalk` shares when it runs, else the config file's.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use masterbus::protocol::{
    COMMIT_TOKEN_BUS_ID, bus_id_for_addr, can_class, encode_commit_as, encode_set_boolean,
    group_count_req_raw, heartbeat_raw, monitoring_req_raw,
};
use masterbus::transport::{TransportRx, TransportTx};
use masterbus::{Config, MasterBus};

const USAGE: &str = "\
usage: masterbus-event-test <device_id> <field_id>

  <device_id>   hex 24-bit address (e.g. 40944B)
  <field_id>    hex field of an on/off output that is safe to switch (e.g. 15)
";

const LOG_FILE: &str = "event-test.log";

/// How long a device gets to act on a write before we read the output back.
/// A relay-style output answers the first read with the old value.
const SETTLE: Duration = Duration::from_secs(3);

/// Our own bus address when the config file names none: the example
/// `heartbeat_master` the config file suggests.
const DEFAULT_OWN_ADDR: u32 = 0x000001;

/// One way of writing the output.
struct Variant {
    label: String,
    /// The writer the commit names, or `None` for a value write alone.
    commit_as: Option<u32>,
}

struct Bus {
    rx: Box<dyn TransportRx>,
    tx: Box<dyn TransportTx>,
    log: File,
    started: Instant,
    heartbeat: Option<u32>,
    last_heartbeat: Instant,
    /// Addresses heard announcing themselves (class `0x04`).
    heard: BTreeSet<u32>,
    /// Frames addressed to an address we watch (our own ids).
    watch: BTreeSet<u32>,
    watched: Vec<String>,
}

impl Bus {
    fn log(&mut self, dir: &str, id: u32, data: &[u8]) -> String {
        let hex: Vec<String> = data.iter().map(|b| format!("{b:02X}")).collect();
        let line = format!(
            "{:9.3} {dir} {id:08X} [{}] {}",
            self.started.elapsed().as_secs_f64(),
            data.len(),
            hex.join(" ")
        );
        let _ = writeln!(self.log, "{line}");
        line
    }

    fn note(&mut self, text: &str) {
        println!("{text}");
        let _ = writeln!(self.log, "# {text}");
    }

    fn send(&mut self, (id, data): (u32, Vec<u8>)) {
        self.log("Tx", id, &data);
        if let Err(e) = self.tx.send(id, &data) {
            self.note(&format!("error: send failed: {e}"));
        }
    }

    /// Take frames for up to `dur`; return the first one `want` accepts.
    fn wait_for(
        &mut self,
        dur: Duration,
        want: impl Fn(u32, &[u8]) -> bool,
    ) -> Option<(u32, Vec<u8>)> {
        let deadline = Instant::now() + dur;
        loop {
            if let Some(master) = self.heartbeat {
                if self.last_heartbeat.elapsed() >= Duration::from_secs(1) {
                    self.last_heartbeat = Instant::now();
                    self.send(heartbeat_raw(master));
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let wait = (deadline - now).min(Duration::from_millis(100));
            match self.rx.recv(wait) {
                Ok(Some((id, data))) => {
                    let line = self.log("Rx", id, &data);
                    let class = (id >> 24) as u8;
                    let addr = id & 0x00FF_FFFF;
                    if class == can_class::DEVICE_BROADCAST {
                        self.heard.insert(addr);
                    }
                    if self.watch.contains(&addr) {
                        self.watched.push(line);
                    }
                    if want(id, &data) {
                        return Some((id, data));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    self.note(&format!("error: {e}"));
                    return None;
                }
            }
        }
    }

    fn listen(&mut self, dur: Duration) {
        self.wait_for(dur, |_, _| false);
    }

    /// Read a Monitoring value; `None` when the device doesn't answer.
    fn read(&mut self, addr: u32, field: u8) -> Option<f32> {
        for _ in 0..3 {
            self.send(monitoring_req_raw(addr, field, 0));
            let want = (u32::from(can_class::MONITORING_DATA) << 24) | addr;
            let got = self.wait_for(Duration::from_millis(600), |id, d| {
                id == want && d.len() >= 6 && d[0] == field && d[1] == 0
            });
            if let Some((_, d)) = got {
                return Some(f32::from_le_bytes([d[2], d[3], d[4], d[5]]));
            }
        }
        None
    }

    /// Ask a device-level property `[0x08, selector]`; the answer's bytes
    /// after the echoed selector.
    fn property(&mut self, addr: u32, selector: u8) -> Option<Vec<u8>> {
        self.send(group_count_req_raw(addr, selector));
        let want = (u32::from(can_class::PROPERTY_INFO) << 24) | addr;
        self.wait_for(Duration::from_millis(400), |id, d| {
            id == want && d.len() >= 2 && d[0] == 0x08 && d[1] == selector
        })
        .map(|(_, d)| d[2..].to_vec())
    }

    /// Write `on` the way `v` says, wait, and read the output back.
    fn switch(&mut self, addr: u32, field: u8, on: bool, v: &Variant) -> Option<bool> {
        self.send(encode_set_boolean(addr, field, on));
        if let Some(bus_id) = v.commit_as {
            self.send(encode_commit_as(addr, field + 1, bus_id));
        }
        self.listen(SETTLE);
        self.read(addr, field).map(|x| x >= 0.5)
    }
}

/// The CAN address of the device with BusID `id`; the inverse of
/// `bus_id_for_addr`.
fn addr_for_bus_id(id: u32) -> u32 {
    ((id >> 24) << 18) | (id & 0x3FFFF)
}

/// Print the frames sent to one of our ids since the last call.
fn report_watched(bus: &mut Bus, prefix: &str) {
    let watched = std::mem::take(&mut bus.watched);
    for line in &watched {
        bus.note(&format!("{prefix}sent to a writer id we used: {line}"));
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(s, 16).ok()
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("--version" | "-V")) {
        println!("masterbus-event-test {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let (addr, field) = match args.as_slice() {
        [d, f] => match (parse_hex_u32(d), parse_hex_u32(f)) {
            (Some(d), Some(f)) if d <= 0x00FF_FFFF && f < 0x100 => (d, f as u8),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let (transport, relay, config) = match MasterBus::joined_transport(Config::default()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: connect failed: {e}");
            return ExitCode::from(3);
        }
    };
    let log = match File::create(LOG_FILE) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: can't write {LOG_FILE} in this folder: {e}");
            return ExitCode::from(3);
        }
    };
    let own_addr = config.heartbeat_master.unwrap_or(DEFAULT_OWN_ADDR);
    // Our own ids: the bus address we heartbeat from, as is and with the top
    // byte of the BusID MasterAdjust's token names (0x14), in case a device
    // only takes commits from that kind of writer.
    let own_id = bus_id_for_addr(own_addr);
    let own_id_14 = (0x14 << 24) | (own_id & 0x00FF_FFFF);
    // Frames to any writer id a commit names, ours or MasterAdjust's.
    let watch: BTreeSet<u32> = [own_id, own_id_14, COMMIT_TOKEN_BUS_ID]
        .into_iter()
        .map(addr_for_bus_id)
        .collect();

    let (rx, tx) = transport.split();
    let mut bus = Bus {
        rx,
        tx,
        log,
        started: Instant::now(),
        heartbeat: config.heartbeat_master,
        last_heartbeat: Instant::now() - Duration::from_secs(1),
        heard: BTreeSet::new(),
        watch,
        watched: Vec::new(),
    };

    bus.note(&format!(
        "masterbus-event-test {}: device {addr:06X} field 0x{field:02X}",
        env!("CARGO_PKG_VERSION")
    ));
    match relay {
        Some(a) => bus.note(&format!("using the bus shared by masterbus-signalk ({a})")),
        None => bus.note("using the bus directly"),
    }
    bus.note("listening for 3 seconds...");
    bus.listen(Duration::from_secs(3));
    let heard: Vec<String> = bus.heard.iter().map(|a| format!("{a:06X}")).collect();
    bus.note(&format!("devices heard: {}", heard.join(" ")));

    let Some(start) = bus.read(addr, field) else {
        bus.note(&format!(
            "error: device {addr:06X} does not answer for field 0x{field:02X}; \
             check the numbers"
        ));
        return ExitCode::from(4);
    };
    if start != 0.0 && start != 1.0 {
        bus.note(&format!(
            "error: field 0x{field:02X} reads {start}, not on or off; pick an on/off output"
        ));
        return ExitCode::from(4);
    }
    let start = start == 1.0;
    bus.note(&format!("output is {} at the start", on_off(start)));

    // The eventable outputs, by the queries the #29 capture shows.
    let count = match bus.property(addr, 0x0D) {
        Some(d) => {
            bus.note(&format!("eventable count [08 0D]: {d:02X?}"));
            d.first().copied().unwrap_or(0)
        }
        None => {
            bus.note("eventable count [08 0D]: no answer");
            0
        }
    };
    // Up to the count when the device gives one, else until it stops
    // answering or answers zero.
    let last = if count > 0 { count.min(0x40) } else { 0x20 };
    let mut outputs = Vec::new();
    for k in 0..last {
        match bus.property(addr, 0x40 + k) {
            Some(d) if count > 0 || d.iter().any(|&b| b != 0) => {
                outputs.push(format!("{k}={d:02X?}"))
            }
            _ => break,
        }
    }
    bus.note(&format!(
        "eventable outputs [08 40+K]: {}",
        if outputs.is_empty() {
            "no answer".to_string()
        } else {
            outputs.join(" ")
        }
    ));

    let variants = [
        Variant {
            label: "value only, no commit".into(),
            commit_as: None,
        },
        Variant {
            label: format!("commit as our own BusID {own_id:08X}"),
            commit_as: Some(own_id),
        },
        Variant {
            label: format!("commit as our own BusID {own_id_14:08X}"),
            commit_as: Some(own_id_14),
        },
        Variant {
            label: format!("commit as MasterAdjust {COMMIT_TOKEN_BUS_ID:08X} (the old way)"),
            commit_as: Some(COMMIT_TOKEN_BUS_ID),
        },
    ];

    report_watched(&mut bus, "   before the tests: ");
    let mut state = start;
    for (i, v) in variants.iter().enumerate() {
        let name = (b'A' + i as u8) as char;
        if let Some(id) = v.commit_as {
            let a = addr_for_bus_id(id);
            if bus.heard.contains(&a) && id != COMMIT_TOKEN_BUS_ID {
                bus.note(&format!(
                    "{name}. {}: skipped, a device on the bus has address {a:06X}",
                    v.label
                ));
                continue;
            }
        }
        let Some(now) = bus.switch(addr, field, !state, v) else {
            bus.note(&format!("{name}. {}: no answer when read back", v.label));
            report_watched(&mut bus, "     ");
            continue;
        };
        if now == state {
            bus.note(&format!(
                "{name}. {}: NO EFFECT (still {})",
                v.label,
                on_off(now)
            ));
        } else {
            state = now;
            let back = bus.switch(addr, field, !state, v);
            if let Some(b) = back {
                state = b;
            }
            bus.note(&format!(
                "{name}. {}: WORKS (switched {}, then {})",
                v.label,
                on_off(now),
                match back {
                    Some(b) if b != now => format!("back {}", on_off(b)),
                    Some(_) => "NOT back".to_string(),
                    None => "no answer".to_string(),
                }
            ));
        }
        report_watched(&mut bus, "     ");
    }

    if state != start {
        bus.note(&format!(
            "putting the output back {} the old way...",
            on_off(start)
        ));
        let old = &variants[3];
        state = bus.switch(addr, field, start, old).unwrap_or(state);
    }
    bus.note("listening for 5 more seconds...");
    bus.listen(Duration::from_secs(5));
    report_watched(&mut bus, "   after the tests: ");
    bus.note(&format!("output is {} at the end", on_off(state)));
    if state != start {
        bus.note(&format!(
            "WARNING: the output is not back to {}; please switch it back by hand",
            on_off(start)
        ));
    }
    println!(
        "\nDone. Please copy everything above into the GitHub issue, and attach the file \
         {LOG_FILE}."
    );
    ExitCode::SUCCESS
}
