//! One-shot CLI to write a single field on a MasterBus device.
//!
//! ```text
//! masterbus-set-field <transport> <device_id> <field_id> <value>
//! ```
//!
//! - `<transport>`: `can0` (Linux SocketCAN) or `usb` (Mastervolt USB link).
//! - `<device_id>`: hex device id from `masterbus-tui`'s title bar — e.g.
//!   `188EA2` or `0x188EA2`.
//! - `<field_id>`: hex field id as shown next to each row in the TUI
//!   (`0x000`..`0x1FF` — bit 8 is the channel: clear=Btm1, set=Btm3).
//! - `<value>`: parsed against the field's discovered visualization type:
//!     - **Boolean**: `true` / `false` / `on` / `off` / `1` / `0`.
//!     - **Float / numeric**: any number.
//!     - **List / DropDown / Eventable**: option index (integer) **or** the
//!       option's exact label string.
//!     - **Text**: the new string content; the current sid is taken from
//!       the cached value.

use std::process::ExitCode;
use std::time::Duration;

use masterbus::{Config, DeviceId, FieldId, MasterBus, Value, VisualizationType};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        eprintln!(
            "usage: masterbus-set-field <transport> <device_id> <field_id> <value>\n\
             \n\
             <transport>   can0 | usb\n\
             <device_id>   hex 24-bit address (e.g. 188EA2)\n\
             <field_id>    hex u16; bit 8 selects Btm1 (0) or Btm3 (1) channel\n\
             <value>       boolean (true/false), number, list index OR option label,\n\
                           or free text — interpreted per the field's type"
        );
        return ExitCode::from(1);
    }
    let transport = &args[0];
    let device_id = match parse_hex_u32(&args[1]) {
        Some(v) if v <= 0x00FF_FFFF => v as DeviceId,
        _ => {
            eprintln!("error: device_id must be a 24-bit hex value (got {:?})", args[1]);
            return ExitCode::from(2);
        }
    };
    let field_id = match parse_hex_u32(&args[2]) {
        Some(v) if v <= u16::MAX as u32 => v as FieldId,
        _ => {
            eprintln!("error: field_id must fit in 16 bits (got {:?})", args[2]);
            return ExitCode::from(2);
        }
    };
    let value_arg = &args[3];

    // Be generous on the connect timeout: we're only doing one round-trip and
    // a quiet bus can take a moment to broadcast.
    let config = Config { connect_timeout: Duration::from_secs(5), ..Default::default() };
    let bus = match build_bus(transport, config) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: connect failed: {e}");
            return ExitCode::from(3);
        }
    };

    let device = bus.device(device_id);
    let field = device.field(field_id);
    let info = match field.info() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: field 0x{field_id:03X} on device 0x{device_id:06X}: {e}");
            return ExitCode::from(4);
        }
    };
    if !info.writeable {
        eprintln!(
            "error: field 0x{field_id:03X} ({}) is read-only at the current access level",
            info.name
        );
        return ExitCode::from(5);
    }

    let value = match build_value(&info.viz_type, &info.options, value_arg, &device, field_id) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(6);
        }
    };

    match field.set(value) {
        Ok(v) => {
            println!("set ok: 0x{device_id:06X} 0x{field_id:03X} {} = {}", info.name, render(&v));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: set failed: {e}");
            ExitCode::from(7)
        }
    }
}

fn build_bus(transport: &str, config: Config) -> masterbus::Result<MasterBus> {
    match transport {
        "usb" => MasterBus::usb(None, config),
        iface => {
            #[cfg(target_os = "linux")]
            {
                MasterBus::socketcan(iface, config)
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = iface;
                Err(masterbus::Error::Connection(
                    "SocketCAN requires Linux; pass `usb` on macOS / Windows".into(),
                ))
            }
        }
    }
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(s, 16).ok()
}

fn build_value(
    viz: &VisualizationType,
    options: &[String],
    arg: &str,
    device: &masterbus::Device,
    field_id: FieldId,
) -> Result<Value, String> {
    use VisualizationType as V;
    match viz {
        V::CheckBox | V::ToggleButton | V::PushButton => {
            let b = match arg.to_ascii_lowercase().as_str() {
                "true" | "on" | "1" | "yes" => true,
                "false" | "off" | "0" | "no" => false,
                _ => return Err(format!("{arg:?} is not a boolean (try true/false/on/off/1/0)")),
            };
            Ok(Value::Boolean(b))
        }
        V::Float | V::GrayVisualization => arg
            .parse::<f32>()
            .map(Value::Float)
            .map_err(|_| format!("{arg:?} is not a number")),
        V::Radio | V::DropDown => {
            let index = resolve_index(arg, options)?;
            Ok(Value::List { index, options: options.to_vec() })
        }
        V::Eventable => {
            let index = resolve_index(arg, options)?;
            Ok(Value::Eventable { index, labels: options.to_vec() })
        }
        V::Text => {
            // The editable sid for a Text field is the field's "value" — the
            // crate decodes the f32 push as `Value::Text { sid, .. }`. Read
            // it once so we have the sid, then write the new text to it.
            let cached = device
                .field(field_id)
                .value()
                .map_err(|e| format!("can't read current value to discover sid: {e}"))?;
            let sid = match cached {
                Value::Text { sid, .. } => sid,
                other => return Err(format!("expected Text-VIZ value, got {other:?}")),
            };
            Ok(Value::Text { sid, text: arg.to_string() })
        }
        V::Date | V::Time => Err("Date / Time fields aren't writable from this CLI".to_string()),
        V::DeviceList => Err("DeviceRef fields aren't writable from this CLI".to_string()),
    }
}

/// Accept either a decimal/hex integer index, or the exact option label.
fn resolve_index(arg: &str, options: &[String]) -> Result<i32, String> {
    if let Some(i) = parse_hex_u32(arg) {
        return Ok(i as i32);
    }
    if let Ok(i) = arg.parse::<i32>() {
        return Ok(i);
    }
    options
        .iter()
        .position(|o| o == arg)
        .map(|i| i as i32)
        .ok_or_else(|| {
            format!(
                "{arg:?} doesn't match any option; available: {}",
                options.join(", ")
            )
        })
}

fn render(v: &Value) -> String {
    match v {
        Value::Float(f) => format!("{f}"),
        Value::Boolean(b) => format!("{b}"),
        Value::Text { text, .. } => format!("{text:?}"),
        Value::List { index, options } => match options.get(*index as usize) {
            Some(s) => format!("{s} ({index})"),
            None => format!("index {index}"),
        },
        other => format!("{other:?}"),
    }
}
