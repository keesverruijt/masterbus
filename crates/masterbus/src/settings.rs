//! Permanentish per-host configuration: where the bus is and whether we drive
//! it as master. Lives in a small INI file so tools don't need transport
//! arguments on every invocation.
//!
//! # Location
//!
//! - **Linux**: `/etc/default/masterbus/config.ini` if present (or creatable in
//!   `/etc/default/masterbus/` / `/etc/default/`); otherwise
//!   `$XDG_CONFIG_HOME/masterbus/config.ini`
//!   (defaults to `$HOME/.config/masterbus/config.ini`).
//! - **macOS**: `$HOME/Library/Application Support/masterbus/config.ini`.
//! - **Windows**: `%APPDATA%\masterbus\config.ini`.
//!
//! On every platform the `MASTERBUS_CONFIG_DIR` environment variable overrides
//! all of the above: `config.ini` is then `$MASTERBUS_CONFIG_DIR/config.ini`,
//! and a config file created there defaults its schema cache to
//! `$MASTERBUS_CONFIG_DIR/cache`. That is how a supervisor such as the Signal K
//! plugin keeps a daemon's whole state in one directory of its own choosing.
//!
//! Everything else this project stores per host lives in that same directory.
//! In particular the Signal K sidecar's field mapping is `mapping.json` beside
//! `config.ini` — see [`FileConfig::mapping_path`]. There is deliberately no
//! second configuration directory.
//!
//! The schema cache (`cache_dir`) follows the same convention:
//!
//! - **Linux**: `/var/lib/masterbus` (system) or `$XDG_CACHE_HOME/masterbus`
//!   (per-user; defaults to `$HOME/.cache/masterbus`).
//! - **macOS**: `$HOME/Library/Caches/masterbus`.
//! - **Windows**: `%LOCALAPPDATA%\masterbus\cache`.
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
use std::path::Path;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::model::DeviceId;

/// Name of the Signal K field-mapping file, kept beside `config.ini`.
pub const MAPPING_FILE: &str = "mapping.json";

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
    /// On-disk schema cache directory; `None` = caching disabled (the key is
    /// absent or commented out in the file).
    pub cache_dir: Option<PathBuf>,
    /// Address `masterbus-signalk` listens on; `None` = the tool's own default.
    /// Kept here so the systemd unit needs no environment file of its own.
    pub listen: Option<String>,
    /// Address `masterbus-signalk` serves its HTTP control API on; `None` =
    /// the API is off. The Signal K plugin talks to this.
    pub api_listen: Option<String>,
    /// Bearer token the control API requires. Optional on a loopback
    /// `api_listen`, mandatory on any other address.
    pub api_token: Option<String>,
    /// Path the file was loaded from / created at.
    pub path: PathBuf,
}

impl FileConfig {
    /// The Signal K field mapping file, always beside `config.ini`.
    ///
    /// Keeping one configuration directory per host is deliberate: whichever
    /// location [`Self::load_or_create`] settled on (system, per-user, or an
    /// OS-native path) is the one the mapping is read from and written to, so
    /// the TUI editor and the sidecar cannot disagree about where it lives.
    pub fn mapping_path(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(MAPPING_FILE)
    }

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
        let mut detected = autodetect()?;
        detected.cache_dir = default_cache_dir(&path);
        let body = render(&detected);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Connection(format!("mkdir {}: {e}", parent.display())))?;
        }
        let mut f = fs::File::create(&path)
            .map_err(|e| Error::Connection(format!("create {}: {e}", path.display())))?;
        f.write_all(body.as_bytes())
            .map_err(|e| Error::Connection(format!("write {}: {e}", path.display())))?;
        log::debug!(
            target: "masterbus::settings",
            "created {} (device_type={:?}, device_name={:?}, cache_dir={:?})",
            path.display(),
            detected.device_type,
            detected.device_name,
            detected.cache_dir,
        );
        Ok(FileConfig { path, ..detected })
    }
}

/// Pick a sensible default schema-cache directory based on where the config
/// file is being created. Mirrors the systemd unit's `StateDirectory` when the
/// system path is chosen, so root-run daemons and user-run tools share schemas.
fn default_cache_dir(config_path: &std::path::Path) -> Option<PathBuf> {
    default_cache_dir_with(config_path, config_dir_override())
}

/// [`default_cache_dir`] with the `MASTERBUS_CONFIG_DIR` override passed in,
/// so it can be exercised without touching the process environment.
fn default_cache_dir_with(
    config_path: &std::path::Path,
    config_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = config_dir {
        Some(dir.join("cache"))
    } else if config_path.starts_with("/etc/") {
        Some(PathBuf::from("/var/lib/masterbus"))
    } else {
        user_cache_dir()
    }
}

/// Resolve the file's `cache_dir` to a usable directory: try the requested
/// path first (creating it if needed); if it's not writable, fall back to the
/// OS-native per-user cache. Returns `None` only if both attempts fail.
///
/// `None` input means the user intentionally disabled caching (the key was
/// commented out or absent), so the caller passes `None` straight through.
pub(crate) fn resolve_cache_dir(requested: &std::path::Path) -> Option<PathBuf> {
    if try_use_dir(requested) {
        return Some(requested.to_path_buf());
    }
    let user_cache = user_cache_dir()?;
    if requested == user_cache {
        // We already tried that.
        return None;
    }
    if try_use_dir(&user_cache) {
        log::debug!(
            target: "masterbus::settings",
            "cache_dir {} not writable, falling back to {}",
            requested.display(),
            user_cache.display()
        );
        return Some(user_cache);
    }
    None
}

/// Best-effort: `mkdir -p` and check we can create a file in the resulting dir.
fn try_use_dir(dir: &std::path::Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".masterbus-write-probe");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Resolve the path the config file should live at. Prefers a system-wide
/// location on Linux when one is readable or writable; otherwise the
/// OS-native per-user path.
fn resolve_path() -> Result<PathBuf> {
    resolve_path_with(config_dir_override())
}

/// [`resolve_path`] with the `MASTERBUS_CONFIG_DIR` override passed in.
fn resolve_path_with(config_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = config_dir {
        return Ok(dir.join("config.ini"));
    }
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
    user_config_path().ok_or_else(|| {
        Error::Connection("could not determine the per-user config path (no HOME?)".into())
    })
}

/// Environment variable that pins the configuration directory on every
/// platform (see the module docs).
pub const CONFIG_DIR_ENV: &str = "MASTERBUS_CONFIG_DIR";

/// The directory `MASTERBUS_CONFIG_DIR` names, if it is set and not blank.
fn config_dir_override() -> Option<PathBuf> {
    let v = std::env::var_os(CONFIG_DIR_ENV)?;
    if v.is_empty() {
        return None;
    }
    Some(PathBuf::from(v))
}

#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// OS-native per-user path for `config.ini`:
///
/// - **Linux**: `$XDG_CONFIG_HOME/masterbus/config.ini`
///   (default `$HOME/.config/masterbus/config.ini`)
/// - **macOS**: `$HOME/Library/Application Support/masterbus/config.ini`
/// - **Windows**: `%APPDATA%\masterbus\config.ini`
fn user_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".config")))?;
        Some(base.join("masterbus").join("config.ini"))
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("masterbus")
                .join("config.ini")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join("AppData").join("Roaming")))
            .map(|d| d.join("masterbus").join("config.ini"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        home_dir().map(|h| h.join(".config").join("masterbus").join("config.ini"))
    }
}

/// OS-native per-user schema-cache directory:
///
/// - **Linux**: `$XDG_CACHE_HOME/masterbus` (default `$HOME/.cache/masterbus`)
/// - **macOS**: `$HOME/Library/Caches/masterbus`
/// - **Windows**: `%LOCALAPPDATA%\masterbus\cache`
fn user_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".cache")))?;
        Some(base.join("masterbus"))
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library").join("Caches").join("masterbus"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join("AppData").join("Local")))
            .map(|d| d.join("masterbus").join("cache"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        home_dir().map(|h| h.join(".cache").join("masterbus"))
    }
}

/// Best-effort writability check: try to create+delete a temp file in `dir`.
#[cfg(target_os = "linux")]
fn is_writable_dir(dir: &Path) -> bool {
    let probe = dir.join(".masterbus-write-probe");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
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
    let mut cache_dir: Option<PathBuf> = None;
    let mut listen: Option<String> = None;
    let mut api_listen: Option<String> = None;
    let mut api_token: Option<String> = None;
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
            "cache_dir" if !value.is_empty() => {
                cache_dir = Some(PathBuf::from(value));
            }
            "listen" if !value.is_empty() => listen = Some(value.to_string()),
            "api_listen" if !value.is_empty() => api_listen = Some(value.to_string()),
            "api_token" if !value.is_empty() => api_token = Some(value.to_string()),
            _ => {} // forward-compat: ignore unknown keys + empty cache_dir
        }
    }
    let device_type = device_type
        .ok_or_else(|| Error::Connection(format!("{}: device_type is required", path.display())))?;
    Ok(FileConfig {
        heartbeat_master,
        device_type,
        device_name,
        cache_dir,
        listen,
        api_listen,
        api_token,
        path,
    })
}

/// Render a `FileConfig` to its on-disk INI form, with explanatory comments.
fn render(cfg: &FileConfig) -> String {
    let hb = match cfg.heartbeat_master {
        Some(v) => format!("heartbeat_master = {:06X}\n", v),
        None => "# heartbeat_master = 000001\n".to_string(),
    };
    let cache = match &cfg.cache_dir {
        Some(p) => format!("cache_dir = {}\n", p.display()),
        None => "# cache_dir = /var/lib/masterbus\n".to_string(),
    };
    let listen = match &cfg.listen {
        Some(a) => format!("listen = {a}\n"),
        None => "# listen = 0.0.0.0:3009\n".to_string(),
    };
    let api_listen = match &cfg.api_listen {
        Some(a) => format!("api_listen = {a}\n"),
        None => "# api_listen = 127.0.0.1:3010\n".to_string(),
    };
    let api_token = match &cfg.api_token {
        Some(t) => format!("api_token = {t}\n"),
        None => "# api_token = change-me\n".to_string(),
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
         device_name = {dn}\n\
         \n\
         # Where to persist discovered schemas (per device, by serial). If the\n\
         # path isn't writable by the running user, the engine falls back to\n\
         # $HOME/.cache/masterbus. Comment out to disable on-disk caching.\n\
         {cache}\n\
         # Address masterbus-signalk listens on. Comment out for its default\n\
         # (0.0.0.0:3009). A command-line argument still wins over this.\n\
         {listen}\n\
         # Address masterbus-signalk serves its HTTP control API on (what the\n\
         # Signal K plugin talks to: devices, the mapping, writes). Comment out\n\
         # to leave the API off. Any address other than loopback also needs\n\
         # api_token, which a client sends as `Authorization: Bearer <token>`.\n\
         {api_listen}{api_token}",
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
            cache_dir: None,
            listen: None,
            api_listen: None,
            api_token: None,
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
                cache_dir: None,
                listen: None,
                api_listen: None,
                api_token: None,
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
            matches!(
                fs::read_to_string(&p).ok().as_deref().map(str::trim),
                Some("280")
            )
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_lives_beside_config_wherever_that_is() {
        // The point of the accessor: one configuration directory per host, so
        // the TUI editor and the sidecar cannot disagree about the location.
        for dir in [
            "/etc/default/masterbus",
            "/home/kees/.config/masterbus",
            "/Users/kees/Library/Application Support/masterbus",
        ] {
            let cfg = parse(
                "device_type = usb\ndevice_name =\n",
                PathBuf::from(dir).join("config.ini"),
            )
            .unwrap();
            assert_eq!(cfg.mapping_path(), PathBuf::from(dir).join("mapping.json"));
        }
    }

    #[test]
    fn listen_is_optional_and_round_trips() {
        let cfg = parse("device_type = usb\ndevice_name =\n", PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.listen, None);
        assert!(render(&cfg).contains("# listen = 0.0.0.0:3009"));

        let raw = "device_type = usb\ndevice_name =\nlisten = 127.0.0.1:4000\n";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.listen.as_deref(), Some("127.0.0.1:4000"));
        let again = parse(&render(&cfg), PathBuf::from("t.ini")).unwrap();
        assert_eq!(again.listen.as_deref(), Some("127.0.0.1:4000"));
    }

    /// The control API is off unless asked for, so an install that predates
    /// it does not silently open a port; and its keys survive a rewrite.
    #[test]
    fn api_keys_are_optional_and_round_trip() {
        let cfg = parse("device_type = usb\ndevice_name =\n", PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.api_listen, None);
        assert_eq!(cfg.api_token, None);
        let rendered = render(&cfg);
        assert!(rendered.contains("# api_listen = 127.0.0.1:3010"));
        assert!(rendered.contains("# api_token = "));

        let raw = "device_type = usb\ndevice_name =\napi_listen = 0.0.0.0:3010\napi_token = abc\n";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.api_listen.as_deref(), Some("0.0.0.0:3010"));
        assert_eq!(cfg.api_token.as_deref(), Some("abc"));
        let again = parse(&render(&cfg), PathBuf::from("t.ini")).unwrap();
        assert_eq!(again.api_listen.as_deref(), Some("0.0.0.0:3010"));
        assert_eq!(again.api_token.as_deref(), Some("abc"));
    }

    /// `MASTERBUS_CONFIG_DIR` pins the whole configuration directory, so a
    /// supervisor can keep a daemon's config, mapping and cache together.
    /// Tested through the `_with` variants: the environment is process-wide
    /// and other tests here read it concurrently.
    #[test]
    fn config_dir_override_pins_config_and_cache_together() {
        let over = Some(PathBuf::from("/tmp/mb-plugin-state"));
        let p = resolve_path_with(over.clone()).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/mb-plugin-state/config.ini"));
        assert_eq!(
            default_cache_dir_with(&p, over),
            Some(PathBuf::from("/tmp/mb-plugin-state/cache"))
        );
        // Without the override the platform rules apply, and a system config
        // still caches in the state directory.
        assert_eq!(
            default_cache_dir_with(Path::new("/etc/default/masterbus/config.ini"), None),
            Some(PathBuf::from("/var/lib/masterbus"))
        );
    }

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
            cache_dir: Some(PathBuf::from("/var/lib/masterbus")),
            listen: Some("0.0.0.0:3009".into()),
            api_listen: Some("0.0.0.0:3010".into()),
            api_token: Some("s3cret".into()),
            path: PathBuf::from("t.ini"),
        };
        let s = render(&cfg);
        let back = parse(&s, PathBuf::from("t.ini")).unwrap();
        assert_eq!(back.api_listen.as_deref(), Some("0.0.0.0:3010"));
        assert_eq!(back.api_token.as_deref(), Some("s3cret"));
        assert_eq!(back.heartbeat_master, Some(1));
        assert_eq!(back.device_type, DeviceType::Can);
        assert_eq!(back.device_name, "can0");
        assert_eq!(back.cache_dir, Some(PathBuf::from("/var/lib/masterbus")));
        assert_eq!(back.listen.as_deref(), Some("0.0.0.0:3009"));
    }

    #[test]
    fn cache_dir_commented_means_disabled() {
        let raw = "device_type = can\n# cache_dir = /var/lib/masterbus\n";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.cache_dir, None);
    }

    #[test]
    fn cache_dir_present() {
        let raw = "device_type = can\ncache_dir = /tmp/cache\n";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.cache_dir, Some(PathBuf::from("/tmp/cache")));
    }

    /// A line that isn't `key = value` names the file and the line number —
    /// a hand-edited config should say where it went wrong.
    #[test]
    fn a_malformed_line_is_rejected_with_its_line_number() {
        let raw = "device_type = can\n\nthis is not ini\n";
        let err = parse(raw, PathBuf::from("t.ini")).unwrap_err().to_string();
        assert!(err.contains("t.ini:3"), "{err}");
        assert!(err.contains("expected `key = value`"), "{err}");
    }

    #[test]
    fn an_unknown_device_type_is_rejected_by_name() {
        let err = parse("device_type = rs232\n", PathBuf::from("t.ini"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("t.ini:1"), "{err}");
        assert!(err.contains("rs232"), "{err}");
    }

    #[test]
    fn socketcan_is_an_alias_for_can() {
        let cfg = parse("device_type = SocketCAN\n", PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.device_type, DeviceType::Can);
    }

    /// Values may be quoted and lines may carry a trailing comment in either
    /// style; neither should end up in the value.
    #[test]
    fn values_are_unquoted_and_stripped_of_trailing_comments() {
        let raw = "\
            device_type = usb   ; the link\n\
            device_name = \"ML2311\"  # its serial\n\
            ";
        let cfg = parse(raw, PathBuf::from("t.ini")).unwrap();
        assert_eq!(cfg.device_type, DeviceType::Usb);
        assert_eq!(cfg.device_name, "ML2311");
    }

    /// Rendering a config that has nothing set leaves every optional key
    /// present but commented out, so the file documents itself.
    #[test]
    fn an_empty_config_renders_its_optional_keys_as_comments() {
        let cfg = FileConfig {
            heartbeat_master: None,
            device_type: DeviceType::Usb,
            device_name: String::new(),
            cache_dir: None,
            listen: None,
            api_listen: None,
            api_token: None,
            path: PathBuf::from("t.ini"),
        };
        let rendered = render(&cfg);
        assert!(
            rendered.contains("# api_listen = 127.0.0.1:3010"),
            "{rendered}"
        );
        assert!(
            rendered.contains("# heartbeat_master = 000001"),
            "{rendered}"
        );
        assert!(
            rendered.contains("# cache_dir = /var/lib/masterbus"),
            "{rendered}"
        );
        assert!(rendered.contains("# listen = 0.0.0.0:3009"), "{rendered}");
        assert!(rendered.contains("device_type = usb"), "{rendered}");

        // And it still parses back to the same nothing.
        let back = parse(&rendered, PathBuf::from("t.ini")).unwrap();
        assert_eq!(back.heartbeat_master, None);
        assert_eq!(back.cache_dir, None);
        assert_eq!(back.listen, None);
    }

    /// A writable directory is used as-is — no fallback, no surprise
    /// relocation of someone's cache.
    #[test]
    fn a_writable_cache_dir_is_used_as_is() {
        let dir = temp_dir();
        let nested = dir.join("schemas");
        assert_eq!(resolve_cache_dir(&nested), Some(nested.clone()));
        assert!(nested.is_dir(), "the directory should have been created");
        // The write probe cleans up after itself.
        assert_eq!(std::fs::read_dir(&nested).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A path that cannot be a directory at all (here: under a regular file)
    /// fails the probe rather than being handed back as usable.
    #[test]
    fn an_unusable_cache_dir_fails_the_probe() {
        let dir = temp_dir();
        let file = dir.join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        assert!(!try_use_dir(&file.join("schemas")));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The default cache location follows the config location: a system-wide
    /// config caches where the systemd unit's StateDirectory points, anything
    /// else caches per user.
    #[test]
    fn default_cache_for_system_path() {
        let sys = default_cache_dir(std::path::Path::new("/etc/default/masterbus/config.ini"));
        assert_eq!(sys, Some(PathBuf::from("/var/lib/masterbus")));

        let user = default_cache_dir(std::path::Path::new(
            "/home/kees/.config/masterbus/config.ini",
        ));
        assert_eq!(user, user_cache_dir());
        assert_ne!(user, sys);
    }

    /// A scratch directory for the cache-probe tests. Removed by each test;
    /// nothing here touches the real per-user cache.
    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "masterbus-settings-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn user_paths_follow_xdg() {
        let cfg = user_config_path().expect("HOME set in tests");
        assert!(
            cfg.ends_with(".config/masterbus/config.ini") || cfg.ends_with("masterbus/config.ini"), // XDG_CONFIG_HOME set
            "unexpected linux user config: {}",
            cfg.display()
        );
        let cache = user_cache_dir().expect("HOME set in tests");
        assert!(
            cache.ends_with(".cache/masterbus") || cache.ends_with("masterbus"),
            "unexpected linux user cache: {}",
            cache.display()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn user_paths_follow_macos_layout() {
        let cfg = user_config_path().expect("HOME set in tests");
        assert!(cfg.ends_with("Library/Application Support/masterbus/config.ini"));
        let cache = user_cache_dir().expect("HOME set in tests");
        assert!(cache.ends_with("Library/Caches/masterbus"));
    }
}
