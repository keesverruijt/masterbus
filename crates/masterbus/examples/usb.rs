//! Explore / read / write the bus over the Mastervolt USB link (HID).
//!
//! Usage:
//!   cargo run --example usb                 # enumerate devices
//!   cargo run --example usb -- dump <addrHex>        # full schema
//!   cargo run --example usb -- read <addrHex> <idx>  # read a field
//!   cargo run --example usb -- write <addrHex> <idx> <float>

use masterbus::{Config, MasterBus, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bus = match MasterBus::usb(None, Config::default()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(2);
        }
    };

    match args.first().map(String::as_str) {
        None => enumerate(&bus),
        Some("dump") => dump(&bus, &args),
        Some("read") => read(&bus, &args),
        Some("write") => write(&bus, &args),
        Some(other) => eprintln!("unknown subcommand: {other}"),
    }
}

fn addr_arg(args: &[String], i: usize) -> u32 {
    u32::from_str_radix(args.get(i).map(String::as_str).unwrap_or(""), 16).unwrap_or_else(|_| {
        eprintln!("expected hex address as arg {i}");
        std::process::exit(1);
    })
}

fn enumerate(bus: &MasterBus) {
    // Phase 1 — RX only: which device addresses do we hear broadcasting?
    let devices = bus.devices_all();
    println!("heard {} device(s) (RX/broadcasts):", devices.len());
    for d in &devices {
        println!("  {:06X}", d.id());
    }
    // Phase 2 — TX+RX: fetch each identity (sends a property request, awaits reply).
    println!("\nidentities (TX+RX):");
    for d in &devices {
        match d.identity() {
            Ok(id) => println!(
                "  {:06X}  {:<22} art={} fw={} ser={}",
                d.id(),
                id.name,
                id.article,
                id.firmware,
                id.serial
            ),
            Err(e) => println!("  {:06X}  <identity error: {e}>", d.id()),
        }
    }
}

fn dump(bus: &MasterBus, args: &[String]) {
    let addr = addr_arg(args, 1);
    let dev = bus.device(addr);
    let name = dev.name().unwrap_or_else(|_| "?".into());
    println!("device {addr:06X}  {name}");
    let schema = match dev.schema() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("schema error: {e}");
            return;
        }
    };
    for g in &schema.groups {
        println!("\n[{:?}] group {} \"{}\"", g.menu, g.id, g.name);
        for f in &g.fields {
            let val = dev.field(f.index).value().ok();
            println!(
                "    #{:<4} {:<24} {:<8} writable={}  viz={:?}  value={:?}",
                f.index, f.name, f.unit, f.writeable, f.viz_type, val
            );
        }
    }
}

fn read(bus: &MasterBus, args: &[String]) {
    let addr = addr_arg(args, 1);
    let idx: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(-1);
    let field = bus.device(addr).field(idx);
    println!(
        "{addr:06X} #{idx} \"{}\" = {:?} (writable={:?})",
        field.name().unwrap_or_default(),
        field.value(),
        field.is_writable()
    );
}

fn write(bus: &MasterBus, args: &[String]) {
    let addr = addr_arg(args, 1);
    let idx: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(-1);
    let v: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        eprintln!("expected float value as arg 3");
        std::process::exit(1);
    });
    let field = bus.device(addr).field(idx);
    let name = field.name().unwrap_or_default();
    println!("{addr:06X} #{idx} \"{name}\" before = {:?}", field.value());
    match field.set(Value::Float(v)) {
        Ok(observed) => println!("wrote {v}; observed = {observed:?}"),
        Err(e) => eprintln!("write failed: {e}"),
    }
    println!("{addr:06X} #{idx} \"{name}\" after  = {:?}", field.value());
}
