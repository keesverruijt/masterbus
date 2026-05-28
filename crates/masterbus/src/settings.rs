//! Permanentish per-host configuration: where the bus is and whether we drive
//! it as master. Lives in a small INI file so tools don't need transport
//! arguments on every invocation.
//!
//! # Location
//!
//! - **Linux**: `/etc/default/masterbus/config.ini` if present (or creatable in
//!   `/etc/default/masterbus/` / `/etc/default/`). Otherwise falls back to
//!   `$HOME/.local/masterbus/config.ini`.
//! - **macOS / Windows**: `$HOME/.local/masterbus/config.ini`
//!   (resp. `%USERPROFILE%\.local\masterbus\config.ini`).
//!
//! # File format
//!
//! ```ini
//! # 24-bit hex device id this host announces as bus master (class-0x05
//! # heartbeats). Comment out to stay passive (a hardware master must drive
//! # the bus).
//! heartbeat_master = 000001
//!
//! # Transport: "usb" for the Mastervolt USB link, "can" for SocketCAN.
//! device_type = can
//!
//! # When device_type = can: interface name (e.g. can0, vcan0).
//! # When device_type = usb: optional USB-link serial number (blank = first).
//! device_name = can0
//! ```
//!
//! On first run the file doesn't exist and is created with auto-detected
//! values: if a Mastervolt USB link is plugged in, `device_type = usb`;
//! otherwise, if exactly one CAN interface exists, `device_type = can` with
//! its name. Multiple CAN interfaces with no USB link is treated as an error —
//! the user is expected to edit the file and pick one.
//!
//! Creation logs the chosen path and detected values to stderr so a first run
//! isn't silent.

use std::fs;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::model::DeviceId;

/// Which transport the file selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    /// SocketCAN (`device_name` is the interface, e.g. `can0`).
    Can,
    /// Mastervolt USB link (`device_name` is the serial number, or blank).
    Usb,
}

/// Parsed/loaded contents of `config.ini`.
#[derive(Debug, Clone)]
pub struct FileConfig {
    /// 24-bit hex master id to announce as, or `None` to stay passive.
    pub heartbeat_master: Option<DeviceId>,
    /// Transport selector.
    pub device_type: DeviceType,
    /// CAN interface name, or USB serial number. Empty string = unspecified.
    pub device_name: String,
    /// Path the file was loaded from / created at.
    pub path: PathBuf,
}

impl FileConfig {
    /// Load the standard config file, creating one with auto-detected values
    /// on first run. Reasons for failure: no writable location to create the
    /// file, or ambiguous hardware (multiple CAN interfaces, no USB link).
    pub fn load_or_create() -> Result<Self> {
        let path = resolve_path()?;
        if path.is_file() {
            let raw = fs::read_to_string(&path)
                .map_err(|e| Error::Connection(format!("read {}: {e}", path.display())))?;
            return parse(&raw, path);
        }
        // Doesn't exist yet — auto-detect and write.
        let detected = autodetect()?;
        let body = render(&detected);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Connection(format!("mkdir {}: {e}", parent.display())))?;
        }
        let mut f = fs::File::create(&path)
            .map_err(|e| Error::Connection(format!("create {}: {e}", path.display())))?;
        f.write_all(body.as_bytes())
            .map_err(|e| Error::Connection(format!("write {}: {e}", path.display())))?;
        eprintln!(
            "masterbus: created {} (device_type={:?}, device_name={:?})",
            path.display(),
            detected.device_type,
            detected.device_name,
        );
        Ok(FileConfig { path, ..detected })
    }
}

/// Resolve the path the config file should live at. Prefers a system-wide
/// location on Linux when one is readable or writable; otherwise the
/// home-directory fallback.
fn resolve_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let sys_dir = Path::new("/etc/default/masterbus");
        let sys_path = sys_dir.join("config.ini");
        if sys_path.is_file() {
            return Ok(sys_path);
        }
        // Decide whether we can create it in the system location.
        let system_writable = if sys_dir.is_dir() {
            is_writable_dir(sys_dir)
        } else {
            is_writable_dir(Path::new("/etc/default"))
        };
        if system_writable {
            return Ok(sys_path);
        }
    }
    let home = home_dir()
        .ok_or_else(|| Error::Connection("no home directory (HOME unset)".into()))?;
    Ok(home.join(".local").join("masterbus").join("config.ini"))
}

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Best-effort writability check: try to create+delete a temp file in `dir`.
#[cfg(target_os = "linux")]
fn is_writable_dir(dir: &Path) -> bool {
    let probe = dir.join(".masterbus-write-probe");
    match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Parse the INI body. Accepts blank lines, `# `/`;` comments, and `key = value`.
/// Unknown keys are ignored (forward compatibility).
fn parse(raw: &str, path: PathBuf) -> Result<FileConfig> {
    let mut heartbeat_master: Option<DeviceId> = None;
    let mut device_type: Option<DeviceType> = None;
    let mut device_name = String::new();
    for (lineno, line) in raw.lines().enumerate() {
        let lineno = lineno + 1;
        let stripped = line.split(['#', ';']).next().unwrap_or("").trim();
        if stripped.is_empty() {
            continue;
        }
        let Some((key, value)) = stripped.split_once('=') else {
            return Err(Error::Connection(format!(
                "{}:{}: expected `key = value`",
                path.display(),
                lineno
            )));
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim();
        match key {
            "heartbeat_master" => {
                let v = u32::from_str_radix(value.trim_start_matches("0x"), 16).map_err(|_| {
                    Error::Connection(format!(
                        "{}:{}: heartbeat_master must be 24-bit hex (got {:?})",
                        path.display(),
                        lineno,
                        value
                    ))
                })?;
                heartbeat_master = Some(v);
            }
            "device_type" => {
                device_type = Some(match value.to_ascii_lowercase().as_str() {
                    "can" | "socketcan" => DeviceType::Can,
                    "usb" => DeviceType::Usb,
                    other => {
                        return Err(Error::Connection(format!(
                            "{}:{}: device_type must be `can` or `usb` (got {:?})",
                            path.display(),
                            lineno,
                            other
                        )));
                    }
                });
            }
            "device_name" => device_name = value.to_string(),
            _ => {} // forward-compat: ignore unknown keys
        }
    }
    let device_type = device_type.ok_or_else(|| {
        Error::Connection(format!("{}: device_type is required", path.display()))
    })?;
    Ok(FileConfig { heartbeat_master, device_type, device_name, path })
}

/// Render a `FileConfig` to its on-disk INI form, with explanatory comments.
fn render(cfg: &FileConfig) -> String {
    let hb = match cfg.heartbeat_master {
        Some(v) => format!("heartbeat_master = {:06X}\n", v),
        None => "# heartbeat_master = 000001\n".to_string(),
    };
    format!(
        "# masterbus configuration.\n\
         #\n\
         # 24-bit hex device id this host announces as bus master\n\
         # (class-0x05 heartbeats). Comment out to stay passive (a hardware\n\
         # master must drive the bus, e.g. an EasyView panel).\n\
         {hb}\n\
         # Transport: \"usb\" for the Mastervolt USB link, \"can\" for SocketCAN.\n\
         device_type = {dt}\n\
         \n\
         # When device_type = can: interface name (e.g. can0, vcan0).\n\
         # When device_type = usb: optional USB-link serial number (blank = first).\n\
         device_name = {dn}\n",
        dt = match cfg.device_type {
            DeviceType::Can => "can",
            DeviceType::Usb => "usb",
        },
        dn = cfg.device_name,
    )
}

/// Look at the hardware and pick sensible defaults: prefer USB if a Mastervolt
/// link is present, else the lone CAN interface. Returns a `FileConfig` with
/// an empty `path` (the caller fills it in).
fn autodetect() -> Result<FileConfig> {
    // First, USB.
    if let Some(serial) = detect_usb_link() {
        return Ok(FileConfig {
            heartbeat_master: None,
            device_type: DeviceType::Usb,
            device_name: serial,
            path: PathBuf::new(),
        });
    }
    // Then CAN (Linux-only).
    #[cfg(target_os = "linux")]
    {
        let cans = list_can_interfaces();
        match cans.as_slice() {
            [] => Err(Error::Connection(
                "no Mastervolt USB link and no CAN interface found — \
                 plug in a USB link or bring up a CAN interface, then re-run"
                    .into(),
            )),
            [one] => Ok(FileConfig {
                heartbeat_master: None,
                device_type: DeviceType::Can,
                device_name: one.clone(),
                path: PathBuf::new(),
            }),
            many => Err(Error::Connection(format!(
                "no Mastervolt USB link found, but multiple CAN interfaces present ({}); \
                 please edit the config file and set `device_name` to the one to use",
                many.join(", ")
            ))),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::Connection(
            "no Mastervolt USB link found (SocketCAN is Linux-only)".into(),
        ))
    }
}

/// Look for a Mastervolt USB Link (vendor 0x1A64). Returns its serial number
/// (empty string if the device has no serial), or `None` if absent.
fn detect_usb_link() -> Option<String> {
    let api = hidapi::HidApi::new().ok()?;
    let info = api.device_list().find(|d| d.vendor_id() == 0x1A64)?;
    Some(info.serial_number().unwrap_or("").to_string())
}

/// List `can*` / `vcan*` interfaces by reading `/sys/class/net`. Anything with
/// a directory containing `type == 280` (ARPHRD_CAN) qualifies; we approximate
/// with name prefix since that matches all in-tree CAN drivers.
#[cfg(target_os = "linux")]
fn list_can_interfaces() -> Vec<String> {
    let dir = match fs::read_dir("/sys/class/net") {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            // Read /sys/class/net/<name>/type; CAN is 280 (ARPHRD_CAN).
            let p = format!("/sys/class/net/{name}/type");
            matches!(fs::read_to_string(&p).ok().as_deref().map(str::trim), Some("280"))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_file() {
        let raw = "\
            # comment\n\
            heartbeat_master = 000001\n\
            device_type = can\n\
            device_name = can0\n\
            ";
        let cfg = parse(raw, PathBuf::from("test.ini")).unwrap();
        assert_eq!(cfg.heartbeat_master, Some(1));
        assert_eq!(cfg.device_type, DeviceType::Can);
        assert_eq!(cfg.device_name, "can0");
    }

    #[test]
    fn heartbeat_optional() {
        let raw = "device_type = usb\ndevice_name =\n";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.heartbeat_master, None);
        assert_eq!(cfg.device_type, DeviceType::Usb);
        assert_eq!(cfg.device_name, "");
    }

    #[test]
    fn unknown_keys_ignored() {
        let raw = "device_type = can\nfuture_thing = yes\n";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.device_type, DeviceType::Can);
    }

    #[test]
    fn bad_heartbeat_rejected() {
        let raw = "device_type = can\nheartbeat_master = nothex\n";
        assert!(parse(raw, PathBuf::from("t.ini")).is_err());
    }

    #[test]
    fn missing_device_type_rejected() {
        let raw = "device_name = can0\n";
        assert!(parse(raw, PathBuf::from("t.ini")).is_err());
    }

    #[test]
    fn render_round_trips() {
        let cfg = FileConfig {
            heartbeat_master: Some(0x000001),
            device_type: DeviceType::Can,
            device_name: "can0".into(),
            path: PathBuf::from("t.ini"),
        };
        let s = render(&cfg);
        let back = parse(&s, PathBuf::from("t.ini")).unwrap();
        assert_eq!(back.heartbeat_master, Some(1));
        assert_eq!(back.device_type, DeviceType::Can);
        assert_eq!(back.device_name, "can0");
    }
}
