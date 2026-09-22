//! A canned bus for `masterbus-signalk --fake-bus`: three devices with enough
//! variety to exercise the stream, the control API and the Signal K plugin
//! with no hardware.
//!
//! - **BAT House** and **BAT Engine**, two batteries of one article, so
//!   "apply to this article" has a target. State of charge, voltage,
//!   current and temperature, in the units real MLI batteries report.
//! - **MSU Inverter**: a battery voltage, a `Device state` enum that cycles
//!   `Standby` → `Inverting` → `Alarm` so notifications fire, and an
//!   `Inverter` checkbox on its Configuration menu that is writable, so a
//!   `"put": true` entry has something to switch.
//!
//! The values drift a little every second, so a client can see the stream
//! move. Built only with the `fake-bus` feature.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use masterbus::fakebus::{Device, FakeBus};
use masterbus::transport::Transport;
use masterbus::{Menu, field_id};

/// Wire visualization codes (see `protocol::viz_from_wire`).
const VIZ_FLOAT: u8 = 0x01;
const VIZ_CHECKBOX: u8 = 0x05;

/// Bus addresses of the three devices.
pub const HOUSE: u32 = 0x1A0001;
/// See [`HOUSE`].
pub const ENGINE: u32 = 0x1A0002;
/// See [`HOUSE`].
pub const INVERTER: u32 = 0x188EA2;

fn battery(addr: u32, serial: &str, name: &str, soc: f32, volts: f32, amps: f32) -> Device {
    let mut d = Device::new()
        .with_identity("66026000", serial, name)
        .with_group(Menu::Monitoring, 0, "Battery", &[0x00, 0x01, 0x02, 0x05])
        .with_field(
            field_id::btm1(0x00),
            "State of charge",
            "%",
            VIZ_FLOAT,
            false,
        )
        .with_field(field_id::btm1(0x01), "Voltage", "V", VIZ_FLOAT, false)
        .with_field(field_id::btm1(0x02), "Current", "A", VIZ_FLOAT, false)
        .with_field(
            field_id::btm1(0x05),
            "Temperature",
            "\u{b0}C",
            VIZ_FLOAT,
            false,
        )
        .with_btm1(0x00, soc)
        .with_btm1(0x01, volts)
        .with_btm1(0x02, amps)
        .with_btm1(0x05, 21.5);
    d.addr = addr;
    d
}

fn inverter() -> Device {
    let mut d = Device::new()
        .with_identity("26024000", "FAKE-INV-1", "MSU Inverter")
        .with_group(Menu::Monitoring, 0, "DC", &[0x06])
        .with_group(Menu::Monitoring, 1, "State", &[0x10])
        .with_group(Menu::Configuration, 2, "Inverter", &[0x13])
        .with_field(field_id::btm1(0x06), "Main battery", "V", VIZ_FLOAT, false)
        .with_list_field(
            field_id::btm1(0x10),
            "Device state",
            &["Standby", "Inverting", "Alarm"],
        )
        .with_field(field_id::btm1(0x13), "Inverter", "", VIZ_CHECKBOX, true)
        .with_btm1(0x06, 25.4)
        .with_btm1(0x10, 0.0)
        .with_btm1(0x13, 1.0);
    d.addr = INVERTER;
    d
}

/// Start the canned bus. Keep the [`FakeBus`] alive for as long as the
/// transport is in use; dropping it stops the devices.
pub fn canned() -> (FakeBus, Box<dyn Transport>) {
    FakeBus::start_many(vec![
        battery(HOUSE, "FAKE-BAT-1", "BAT House", 87.0, 25.6, -12.3),
        battery(ENGINE, "FAKE-BAT-2", "BAT Engine", 100.0, 26.9, 0.4),
        inverter(),
    ])
}

/// Make the values move: the house battery discharges slowly, the engine
/// battery floats, and the inverter's state cycles every few seconds. Runs
/// on its own thread for as long as the bus lives.
pub fn animate(bus: &FakeBus) {
    let devices: Vec<Arc<_>> = bus.devices.clone();
    thread::Builder::new()
        .name("fake-animate".into())
        .spawn(move || {
            let mut tick: u32 = 0;
            loop {
                thread::sleep(Duration::from_secs(1));
                tick = tick.wrapping_add(1);
                let phase = (tick % 60) as f32 / 60.0;
                let wave = (phase * std::f32::consts::TAU).sin();
                for d in &devices {
                    let mut d = d.lock().unwrap();
                    match d.addr {
                        HOUSE => {
                            d.btm1.insert(0x00, (87.0 - phase * 2.0).to_le_bytes());
                            d.btm1.insert(0x01, (25.6 + wave * 0.2).to_le_bytes());
                            d.btm1.insert(0x02, (-12.3 + wave * 3.0).to_le_bytes());
                        }
                        ENGINE => {
                            d.btm1.insert(0x01, (26.9 + wave * 0.05).to_le_bytes());
                        }
                        INVERTER => {
                            // Standby for 20 s, inverting for 30 s, alarm for 10 s.
                            let state = match tick % 60 {
                                0..20 => 0.0f32,
                                20..50 => 1.0,
                                _ => 2.0,
                            };
                            d.btm1.insert(0x10, state.to_le_bytes());
                            d.btm1.insert(0x06, (25.4 + wave * 0.3).to_le_bytes());
                        }
                        _ => {}
                    }
                }
            }
        })
        .expect("spawn fake animator");
}
