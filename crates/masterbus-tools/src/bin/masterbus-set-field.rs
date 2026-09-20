//! One-shot CLI to write a single field on a MasterBus device.
//!
//! ```text
//! masterbus-set-field <device_id> <field_id> <value>
//! ```
//!
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
//!
//! Transport (USB / SocketCAN) and master role come from the per-host config
//! file (see `masterbus::FileConfig`); the file is created on first run.

use std::process::ExitCode;
use std::time::Duration;

use masterbus::{Config, DeviceId, FieldId, MasterBus, Value, VisualizationType};

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!(
            "usage: masterbus-set-field <device_id> <field_id> <value>\n\
             \n\
             <device_id>   hex 24-bit address (e.g. 188EA2)\n\
             <field_id>    hex u16; bit 8 selects Btm1 (0) or Btm3 (1) channel\n\
             <value>       boolean (true/false), number, list index OR option label,\n\
                           or free text — interpreted per the field's type\n\
             \n\
             Transport + heartbeat-master role come from the config file\n\
             (see `masterbus::FileConfig` for the location and format)."
        );
        return ExitCode::from(1);
    }
    let device_id = match parse_hex_u32(&args[0]) {
        Some(v) if v <= 0x00FF_FFFF => v as DeviceId,
        _ => {
            eprintln!(
                "error: device_id must be a 24-bit hex value (got {:?})",
                args[0]
            );
            return ExitCode::from(2);
        }
    };
    let field_id = match parse_hex_u32(&args[1]) {
        Some(v) if v <= u16::MAX as u32 => v as FieldId,
        _ => {
            eprintln!("error: field_id must fit in 16 bits (got {:?})", args[1]);
            return ExitCode::from(2);
        }
    };
    let value_arg = &args[2];

    // Be generous on the connect timeout: we're only doing one round-trip and
    // a quiet bus can take a moment to broadcast.
    let config = Config {
        connect_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    let bus = match MasterBus::auto(config) {
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
            println!(
                "set ok: 0x{device_id:06X} 0x{field_id:03X} {} = {}",
                info.name,
                render(&v)
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: set failed: {e}");
            ExitCode::from(7)
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
                _ => {
                    return Err(format!(
                        "{arg:?} is not a boolean (try true/false/on/off/1/0)"
                    ));
                }
            };
            Ok(Value::Boolean(b))
        }
        V::Float | V::GrayVisualization => arg
            .parse::<f32>()
            .map(Value::Float)
            .map_err(|_| format!("{arg:?} is not a number")),
        // Radio/DropDown and the event-command selector are all a list index.
        V::Radio | V::DropDown | V::EventCommand => {
            let index = resolve_index(arg, options)?;
            Ok(Value::List {
                index,
                options: options.to_vec(),
            })
        }
        V::Eventable => {
            let index = resolve_index(arg, options)?;
            Ok(Value::Eventable {
                index,
                labels: options.to_vec(),
            })
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
            Ok(Value::Text {
                sid,
                text: arg.to_string(),
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids are taken in the same hex form the TUI prints, with or without the
    /// `0x` prefix — copy-pasting from the screen has to work.
    #[test]
    fn hex_ids_are_accepted_with_or_without_a_prefix() {
        for s in ["188EA2", "0x188EA2", "0X188ea2", " 188EA2 "] {
            assert_eq!(parse_hex_u32(s), Some(0x188EA2), "{s:?}");
        }
        assert_eq!(parse_hex_u32("0x1FF"), Some(0x1FF));
        assert_eq!(parse_hex_u32(""), None);
        assert_eq!(parse_hex_u32("zzz"), None);
        assert_eq!(parse_hex_u32("-1"), None);
    }

    /// A list pick may be given as an index or as the option's exact label,
    /// so a user can write what they see on screen.
    #[test]
    fn a_list_pick_accepts_an_index_or_a_label() {
        let options = vec!["Off".to_string(), "On".to_string(), "Auto".to_string()];
        assert_eq!(resolve_index("2", &options), Ok(2));
        assert_eq!(resolve_index("Auto", &options), Ok(2));
        assert_eq!(resolve_index("Off", &options), Ok(0));
        // An index the device might accept but this build has no label for.
        assert_eq!(resolve_index("9", &options), Ok(9));
    }

    /// A label that matches nothing lists what was available — the user is
    /// usually one typo away.
    #[test]
    fn an_unknown_label_reports_the_available_options() {
        let options = vec!["Off".to_string(), "On".to_string()];
        let err = resolve_index("auto", &options).unwrap_err();
        assert!(err.contains("\"auto\""), "{err}");
        assert!(err.contains("Off, On"), "{err}");
    }

    /// Matching is exact: a label differing only in case is not silently
    /// accepted as a different option's index.
    #[test]
    fn label_matching_is_exact() {
        let options = vec!["Standby".to_string(), "Activated".to_string()];
        assert_eq!(resolve_index("Activated", &options), Ok(1));
        assert!(resolve_index("activated", &options).is_err());
    }

    #[test]
    fn the_confirmation_line_renders_each_value_kind() {
        assert_eq!(render(&Value::Float(13.2)), "13.2");
        assert_eq!(render(&Value::Boolean(true)), "true");
        assert_eq!(
            render(&Value::Text {
                sid: 1,
                text: "Nav Chg".into()
            }),
            "\"Nav Chg\""
        );
        assert_eq!(
            render(&Value::List {
                index: 1,
                options: vec!["Off".into(), "On".into()]
            }),
            "On (1)"
        );
        // A list whose labels we don't have still reports the wire value.
        assert_eq!(
            render(&Value::List {
                index: 4,
                options: vec![]
            }),
            "index 4"
        );
    }
}
