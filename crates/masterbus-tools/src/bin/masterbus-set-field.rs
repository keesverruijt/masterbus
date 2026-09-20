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
//!     - **List / DropDown / Eventable**: the option's exact label, a decimal
//!       index, or a `0x`-prefixed hex index — in that order of preference,
//!       so a label like `AC` is a label and a bare `12` is twelve. The TUI
//!       prints both forms beside each option as `label(index)`.
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

/// Resolve a list/enum pick: the option's exact label, a decimal index, or a
/// `0x`-prefixed hex index.
///
/// The order matters, and getting it wrong wrote the wrong value to a device.
/// Labels come first because they are a closed, known set — an exact match is
/// unambiguous intent, and a label that happens to read as hex (`AC`, `DC`, a
/// bank called `B`) must not be taken for a number. Decimal comes next
/// because that is the form `masterbus-tui` prints beside each option, so
/// what you read off the screen is what you can type. Hex is accepted only
/// when it says so: a bare `10` is ten, not sixteen.
fn resolve_index(arg: &str, options: &[String]) -> Result<i32, String> {
    if let Some(i) = options.iter().position(|o| o == arg) {
        return Ok(i as i32);
    }
    if let Ok(i) = arg.parse::<i32>() {
        return Ok(i);
    }
    if let Some(hex) = arg
        .trim()
        .strip_prefix("0x")
        .or_else(|| arg.trim().strip_prefix("0X"))
        && let Ok(i) = i32::from_str_radix(hex, 16)
    {
        return Ok(i);
    }
    Err(format!(
        "{arg:?} is not an option label, a decimal index or a 0x-prefixed hex \
         index; available: {}",
        options.join(", ")
    ))
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

    /// An option's exact label resolves to its index, whatever the label
    /// looks like. `AC` and `DC` are ordinary MasterBus enum labels and also
    /// valid hex; before the parse order was fixed they resolved to 172 and
    /// 220 and were written to the device as such.
    #[test]
    fn a_label_is_a_label_even_when_it_reads_as_hex() {
        let options = vec![
            "Off".to_string(),
            "AC".to_string(),
            "DC".to_string(),
            "Auto".to_string(),
        ];
        assert_eq!(resolve_index("Off", &options), Ok(0));
        assert_eq!(resolve_index("AC", &options), Ok(1));
        assert_eq!(resolve_index("DC", &options), Ok(2));
        assert_eq!(resolve_index("Auto", &options), Ok(3));
    }

    /// A bare number is decimal — the form the TUI prints beside each option
    /// as `label(index)`. Reading `Stabilizer(12)` off the screen and typing
    /// 12 used to write 18.
    #[test]
    fn a_bare_number_is_decimal_not_hex() {
        let options: Vec<String> = Vec::new();
        for (arg, want) in [
            ("0", 0),
            ("2", 2),
            ("9", 9),
            ("10", 10),
            ("12", 12),
            ("24", 24),
            ("99", 99),
        ] {
            assert_eq!(resolve_index(arg, &options), Ok(want), "{arg}");
        }
    }

    /// Hex still works, but only when it says so.
    #[test]
    fn hex_indices_need_their_prefix() {
        let options: Vec<String> = Vec::new();
        assert_eq!(resolve_index("0x12", &options), Ok(18));
        assert_eq!(resolve_index("0X1A", &options), Ok(26));
        assert_eq!(resolve_index("0xff", &options), Ok(255));
    }

    /// Labels that are themselves numbers — a "12 / 24 / 48 V system" enum —
    /// match as labels, so the on-screen text keeps working even though it
    /// collides with the index form.
    #[test]
    fn a_numeric_label_still_matches_as_a_label() {
        let options = vec!["12".to_string(), "24".to_string(), "48".to_string()];
        assert_eq!(resolve_index("24", &options), Ok(1));
        // An index no label spells is still taken as an index.
        assert_eq!(resolve_index("2", &options), Ok(2));
    }

    /// An index this build has no label for is still passed through: the
    /// device's option list may be longer than what discovery resolved.
    #[test]
    fn an_index_beyond_the_known_labels_is_passed_through() {
        let options = vec!["Off".to_string(), "On".to_string()];
        assert_eq!(resolve_index("9", &options), Ok(9));
    }

    /// Matching is exact: a label differing only in case is not silently
    /// accepted as something else.
    #[test]
    fn label_matching_is_exact() {
        let options = vec!["Standby".to_string(), "Activated".to_string()];
        assert_eq!(resolve_index("Activated", &options), Ok(1));
        assert!(resolve_index("activated", &options).is_err());
    }

    /// Anything else names the forms it accepts and lists the options — the
    /// user is usually one typo, or one missing `0x`, away.
    #[test]
    fn an_unresolvable_pick_reports_the_accepted_forms_and_options() {
        let options = vec!["Off".to_string(), "On".to_string()];
        let err = resolve_index("1A", &options).unwrap_err();
        assert!(err.contains("\"1A\""), "{err}");
        assert!(err.contains("0x-prefixed hex"), "{err}");
        assert!(err.contains("Off, On"), "{err}");
        // A hex body that isn't hex at all.
        assert!(resolve_index("0xzz", &options).is_err());
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
