//! The HTTP control API `masterbus-signalk` serves to the Signal K plugin.
//!
//! The delta stream (newline-delimited JSON over TCP) says what the bus is
//! doing. This API is for everything else the plugin needs: which devices and
//! fields exist, what the mapping is, whether a proposed entry would work,
//! and writing a value to a field when Signal K receives a PUT. The contract
//! is documented in `docs/API.md`; [`API_VERSION`] is bumped whenever a
//! response shape changes incompatibly, and the plugin checks it.
//!
//! Synchronous and thread-per-request, on `tiny_http`, like the rest of the
//! engine. Handlers are pure functions of [`Shared`] plus a [`Request`], so
//! they are tested without a socket; one test drives the real server for the
//! transport and the bearer check.
//!
//! Bus access goes through [`BusOps`], which the daemon implements with a
//! [`MasterBus`] and tests implement with a table.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use masterbus::{
    AccessLevel, DeviceId, Error, FieldId, FieldInfo, GroupInfo, MasterBus, Menu, Value,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::mapping::{
    CopyTarget, FieldMapping, Mapping, copy_to_targets, field_key, parse_field_key,
};
use crate::publish::{self, DeviceRec, Diagnostic, Severity, menu_by_name, menu_name};
use crate::seed;
use crate::signalk;
use crate::units::Conversion;

/// How long a device's answer that it has no eventable outputs is kept
/// before it is asked again.
const NO_OUTPUTS_KEPT: Duration = Duration::from_secs(300);

/// The version of the API's shapes. The plugin refuses to talk to a daemon
/// whose `apiVersion` it does not know.
pub const API_VERSION: u32 = 1;

/// The bus operations the API needs, so handlers can be exercised without a
/// bus.
pub trait BusOps: Send + Sync {
    /// Discover one menu of a device and return its groups.
    fn discover_menu(&self, id: DeviceId, menu: Menu) -> masterbus::Result<Vec<GroupInfo>>;
    /// One field's metadata, asked of the device if no discovered menu lists
    /// it; `None` when the device doesn't know the field.
    fn field_info(&self, id: DeviceId, field: FieldId) -> masterbus::Result<Option<FieldInfo>>;
    /// Read a field's current value (cache if fresh, else the bus).
    fn read(&self, id: DeviceId, field: FieldId) -> masterbus::Result<Value>;
    /// Write a field and return the value observed afterwards.
    fn write(&self, id: DeviceId, field: FieldId, value: Value) -> masterbus::Result<Value>;
    /// Log a device in at a level; returns the level it reports afterwards.
    fn login(&self, id: DeviceId, level: AccessLevel, code: f32) -> masterbus::Result<AccessLevel>;
    /// The field ids of a device's eventable outputs, as the device lists them.
    fn eventable_outputs(&self, id: DeviceId) -> masterbus::Result<Vec<FieldId>>;
    /// Switch an eventable output the way an event does; returns the value
    /// observed afterwards.
    fn set_output(&self, id: DeviceId, field: FieldId, on: bool) -> masterbus::Result<Value>;
    /// Whether the link to the bus is there (the USB link can be unplugged).
    fn link_connected(&self) -> bool {
        true
    }
}

impl BusOps for MasterBus {
    fn discover_menu(&self, id: DeviceId, menu: Menu) -> masterbus::Result<Vec<GroupInfo>> {
        self.device(id).tab_info(menu)
    }
    fn field_info(&self, id: DeviceId, field: FieldId) -> masterbus::Result<Option<FieldInfo>> {
        self.device(id).field_info(field)
    }
    fn read(&self, id: DeviceId, field: FieldId) -> masterbus::Result<Value> {
        self.device(id).field(field).value()
    }
    fn write(&self, id: DeviceId, field: FieldId, value: Value) -> masterbus::Result<Value> {
        self.device(id).field(field).set(value)
    }
    fn login(&self, id: DeviceId, level: AccessLevel, code: f32) -> masterbus::Result<AccessLevel> {
        self.device(id).login(level, code)
    }
    fn eventable_outputs(&self, id: DeviceId) -> masterbus::Result<Vec<FieldId>> {
        self.device(id).eventable_output_fields()
    }
    fn set_output(&self, id: DeviceId, field: FieldId, on: bool) -> masterbus::Result<Value> {
        self.device(id).field(field).set_output(on)
    }
    fn link_connected(&self) -> bool {
        MasterBus::link_connected(self)
    }
}

/// Static facts about this daemon, for `/api/status`.
#[derive(Debug, Clone, Default)]
pub struct Info {
    /// `can:can0`, `usb`, `usb:<serial>`.
    pub transport: String,
    /// Where the delta stream listens.
    pub stream: String,
    /// Where this API listens.
    pub api: String,
}

/// The state the daemon and the API share. The daemon's main loop owns the
/// bus and updates `devices`, `diagnostics`, `values` and the counters; the
/// API reads them and replaces `mapping`, then signals `reload`.
pub struct Shared {
    /// The bus, behind the operations the API needs.
    pub bus: Box<dyn BusOps>,
    /// Every device discovered so far.
    pub devices: Mutex<Vec<DeviceRec>>,
    /// The mapping in force.
    pub mapping: Mutex<Mapping>,
    /// What the last activation had to say about the mapping.
    pub diagnostics: Mutex<Vec<Diagnostic>>,
    /// The last value the stream saw per mapped field.
    pub values: Mutex<HashMap<(DeviceId, FieldId), Value>>,
    /// Where the mapping is persisted; `None` keeps it in memory only.
    pub mapping_path: Option<PathBuf>,
    /// Told after every mapping change, so the daemon re-activates.
    pub reload: Sender<()>,
    /// When the daemon started.
    pub started: Instant,
    /// Static facts for `/api/status`.
    pub info: Info,
    /// The bearer token every request must carry, if any.
    pub token: Option<String>,
    /// Fields the stream is currently subscribed to.
    pub streaming: AtomicUsize,
    /// Stream clients currently connected.
    pub clients: AtomicUsize,
    /// Fields already asked of a device on their own (see `ensure_menus`),
    /// so one the device doesn't have is not asked again on every reload.
    pub asked: Mutex<HashSet<(DeviceId, FieldId)>>,
    /// Each device's eventable outputs once asked (see `ensure_outputs`),
    /// and when.
    pub outputs: Mutex<HashMap<DeviceId, (Vec<FieldId>, Instant)>>,
    /// When the devices were identified and started streaming.
    pub timeline: Mutex<Timeline>,
}

/// How the start went, measured from the daemon's start: when the first
/// pass over the bus ended, and per device when it was identified and when
/// its mapped fields started streaming. With these a slow start can be told
/// apart from one slow device, without guessing from the Signal K side (#40).
#[derive(Debug, Default)]
pub struct Timeline {
    /// When the first pass over the bus ended; `None` while it runs.
    pub walked: Option<Duration>,
    /// Per device, once it is identified.
    pub devices: HashMap<DeviceId, DeviceTimes>,
}

/// One device's line in the [`Timeline`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeviceTimes {
    /// When its identity (and, if mapped, its PUT targets) was known.
    pub identified: Duration,
    /// How long identifying it took, from the first request to it.
    pub took: Duration,
    /// Whether it was first heard after the first pass over the bus.
    pub late: bool,
    /// When its mapped fields were first subscribed; `None` while none are.
    pub streaming: Option<Duration>,
}

/// Seconds to a tenth, for the API and the log.
pub fn secs(d: Duration) -> f64 {
    (d.as_secs_f64() * 10.0).round() / 10.0
}

impl Shared {
    /// Persist the mapping, when there is somewhere to persist it.
    fn persist(&self, m: &Mapping) -> Result<(), String> {
        match &self.mapping_path {
            Some(p) => m
                .save(p)
                .map_err(|e| format!("could not write {}: {e}", p.display())),
            None => Ok(()),
        }
    }

    /// A mapping that names a field a device record has not discovered yet
    /// is most likely pointing at a Configuration setting (a PUT target).
    /// Discover that menu once per device, so the entry can be honoured
    /// rather than reported as "no such field". Bus access happens outside
    /// the lock; a device that will not enumerate is logged and left alone.
    pub fn ensure_menus(&self, mapping: &Mapping) {
        let wanted: Vec<(DeviceId, String, String)> = self
            .devices
            .lock()
            .unwrap()
            .iter()
            .filter(|d| {
                !d.menus.contains(&Menu::Configuration)
                    && !publish::unknown_fields(d, mapping).is_empty()
            })
            .map(|d| (d.id, d.name.clone(), d.serial.clone()))
            .collect();
        for (id, name, serial) in wanted {
            match self.bus.discover_menu(id, Menu::Configuration) {
                Ok(groups) => {
                    eprintln!(
                        "masterbus-signalk: {name} ({serial}): mapping names a field outside \
                         Monitoring; discovered its Configuration menu"
                    );
                    let mut devices = self.devices.lock().unwrap();
                    if let Some(d) = devices.iter_mut().find(|d| d.id == id) {
                        d.merge_groups(Menu::Configuration, groups);
                    }
                }
                Err(e) => eprintln!(
                    "masterbus-signalk: {name} ({serial}): could not discover Configuration: {e}"
                ),
            }
        }
        self.ensure_fields(mapping);
    }

    /// A field the mapping names that neither Monitoring nor Configuration
    /// lists is usually one shown only at a higher access level: the mapping
    /// was made in `masterbus-tui` after an installer login, while the daemon
    /// runs at End User level. Such a field still answers its metadata and
    /// value reads, so it is asked for on its own. Each field is asked once.
    fn ensure_fields(&self, mapping: &Mapping) {
        let wanted: Vec<(DeviceId, String, String, Vec<FieldId>)> = {
            let asked = self.asked.lock().unwrap();
            self.devices
                .lock()
                .unwrap()
                .iter()
                .map(|d| {
                    let ids = publish::unknown_fields(d, mapping)
                        .into_iter()
                        .filter(|f| !asked.contains(&(d.id, *f)))
                        .collect::<Vec<_>>();
                    (d.id, d.name.clone(), d.serial.clone(), ids)
                })
                .filter(|(.., ids)| !ids.is_empty())
                .collect()
        };
        for (id, name, serial, ids) in wanted {
            let mut found = Vec::new();
            for field in &ids {
                self.asked.lock().unwrap().insert((id, *field));
                match self.bus.field_info(id, *field) {
                    Ok(Some(f)) => found.push(f),
                    Ok(None) => {}
                    Err(e) => eprintln!(
                        "masterbus-signalk: {name} ({serial}): could not ask for field \
                         0x{field:03X}: {e}"
                    ),
                }
            }
            if found.is_empty() {
                continue;
            }
            eprintln!(
                "masterbus-signalk: {name} ({serial}): {} of the {} mapped field(s) no menu \
                 lists at this access level answered on their own",
                found.len(),
                ids.len()
            );
            let mut devices = self.devices.lock().unwrap();
            if let Some(d) = devices.iter_mut().find(|d| d.id == id) {
                d.merge_groups(
                    publish::MENU,
                    vec![GroupInfo {
                        id: -1,
                        name: String::new(),
                        menu: publish::MENU,
                        fields: found,
                    }],
                );
            }
        }
    }

    /// Ask a device for its eventable outputs, and add the ones no menu
    /// lists to its fields, so the editor can map them. An APR's
    /// `Stop charge` is such an output (#29). A list is kept; no outputs at
    /// all is kept for [`NO_OUTPUTS_KEPT`], since a device that was asleep
    /// may list them later, while a device that has none would otherwise
    /// cost a timeout on every ask. Bus access happens outside the lock.
    fn ensure_outputs(&self, id: DeviceId) {
        if let Some((o, at)) = self.outputs.lock().unwrap().get(&id)
            && (!o.is_empty() || at.elapsed() < NO_OUTPUTS_KEPT)
        {
            return;
        }
        let outputs = match self.bus.eventable_outputs(id) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("masterbus-signalk: 0x{id:06X}: could not list its outputs: {e}");
                return;
            }
        };
        let unlisted: Vec<FieldId> = {
            let devices = self.devices.lock().unwrap();
            let Some(d) = devices.iter().find(|d| d.id == id) else {
                return;
            };
            outputs
                .iter()
                .copied()
                .filter(|f| d.field(*f).is_none())
                .collect()
        };
        let found: Vec<FieldInfo> = unlisted
            .into_iter()
            .filter_map(|f| self.bus.field_info(id, f).ok().flatten())
            .collect();
        if !found.is_empty() {
            let mut devices = self.devices.lock().unwrap();
            if let Some(d) = devices.iter_mut().find(|d| d.id == id) {
                d.merge_groups(
                    publish::MENU,
                    vec![GroupInfo {
                        id: -1,
                        name: "Outputs".into(),
                        menu: publish::MENU,
                        fields: found,
                    }],
                );
            }
        }
        self.outputs
            .lock()
            .unwrap()
            .insert(id, (outputs, Instant::now()));
    }

    /// Whether `field` is one of the device's eventable outputs, asking the
    /// device if it has not been asked yet.
    fn is_output(&self, id: DeviceId, field: FieldId) -> bool {
        self.ensure_outputs(id);
        self.outputs
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|(o, _)| o.contains(&field))
    }

    /// Whether a PUT on this field must switch it the way an event does,
    /// asking the device for its outputs if it has not been asked yet. A
    /// field whose options have no labels always is, whether or not the
    /// device listed it: if the list could not be had (a busy bus), an APR's
    /// writable `Stop charge` must still not take the ordinary write. The
    /// switch itself then refuses a field the device does not list, so it
    /// fails closed.
    fn event_only(&self, id: DeviceId, f: &crate::publish::FieldRec) -> bool {
        signalk::unlabeled(&f.options) || (event_only(f) && self.is_output(id, f.id))
    }

    /// Put a new mapping in force: persist, replace, re-resolve, tell the
    /// daemon. Returns what resolving it had to say.
    fn adopt(&self, m: Mapping) -> Result<publish::Resolved, String> {
        self.persist(&m)?;
        self.ensure_menus(&m);
        let resolved = {
            let devices = self.devices.lock().unwrap();
            publish::resolve(&devices, &m)
        };
        *self.mapping.lock().unwrap() = m;
        *self.diagnostics.lock().unwrap() = resolved.diagnostics.clone();
        let _ = self.reload.send(());
        Ok(resolved)
    }
}

/// An HTTP request, reduced to what the handlers look at.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// `GET`, `PUT`, `POST`.
    pub method: String,
    /// The path without the query string, e.g. `/api/devices/R516V1070`.
    pub path: String,
    /// The query string, split.
    pub query: HashMap<String, String>,
    /// The body, if any.
    pub body: Vec<u8>,
    /// The token from an `Authorization: Bearer` header.
    pub bearer: Option<String>,
}

impl Request {
    /// A request from its method, URL (`path?query`) and body.
    pub fn new(method: &str, url: &str, body: &[u8]) -> Request {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        Request {
            method: method.to_ascii_uppercase(),
            path: path.to_string(),
            query: query
                .split('&')
                .filter(|kv| !kv.is_empty())
                .map(|kv| {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    (k.to_string(), v.to_string())
                })
                .collect(),
            body: body.to_vec(),
            bearer: None,
        }
    }

    /// With a bearer token.
    pub fn bearer(mut self, token: &str) -> Request {
        self.bearer = Some(token.to_string());
        self
    }
}

/// An HTTP response: a status and a JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// The body.
    pub body: Json,
}

impl Response {
    fn ok(body: Json) -> Response {
        Response { status: 200, body }
    }
    fn error(status: u16, message: impl Into<String>) -> Response {
        Response {
            status,
            body: json!({ "error": message.into() }),
        }
    }
}

/// Answer one request. The bearer check comes first; then the path.
pub fn handle(shared: &Shared, req: &Request) -> Response {
    if let Some(t) = &shared.token
        && req.bearer.as_deref() != Some(t.as_str())
    {
        return Response::error(401, "missing or wrong bearer token");
    }
    let Some(rest) = req.path.strip_prefix("/api/") else {
        return Response::error(404, "not an API path");
    };
    let seg: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    match (req.method.as_str(), seg.as_slice()) {
        ("GET", ["status"]) => status(shared),
        ("GET", ["devices"]) => devices(shared),
        ("GET", ["devices", serial]) => device(shared, serial, req.query.get("menu")),
        ("GET", ["devices", serial, "fields", key, "value"]) => read_field(shared, serial, key),
        ("PUT", ["devices", serial, "fields", key]) => write_field(shared, serial, key, &req.body),
        ("GET", ["mapping"]) => Response::ok(json!(*shared.mapping.lock().unwrap())),
        ("PUT", ["mapping"]) => put_mapping(shared, &req.body),
        ("GET", ["mapping", "diagnostics"]) => {
            Response::ok(json!(*shared.diagnostics.lock().unwrap()))
        }
        ("POST", ["mapping", "suggest"]) => suggest(shared, &req.body),
        ("POST", ["mapping", "validate"]) => validate(shared, &req.body),
        ("POST", ["mapping", "apply-article"]) => apply_article(shared, &req.body),
        _ => Response::error(404, format!("no {} {}", req.method, req.path)),
    }
}

fn status(shared: &Shared) -> Response {
    let devices = shared.devices.lock().unwrap();
    let mapping = shared.mapping.lock().unwrap();
    let diagnostics = shared.diagnostics.lock().unwrap();
    let count = |s: Severity| diagnostics.iter().filter(|d| d.severity == s).count();
    Response::ok(json!({
        "apiVersion": API_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "transport": shared.info.transport,
        "stream": shared.info.stream,
        "api": shared.info.api,
        "uptime": shared.started.elapsed().as_secs(),
        "link": link_name(shared.bus.link_connected()),
        "devices": devices.len(),
        "mapped": { "devices": mapping.devices.len(), "fields": mapping.len() },
        "streaming": shared.streaming.load(Ordering::Relaxed),
        "clients": shared.clients.load(Ordering::Relaxed),
        "diagnostics": { "errors": count(Severity::Error), "warnings": count(Severity::Warning) },
        "walked": shared.timeline.lock().unwrap().walked.map(secs),
    }))
}

/// The link's state as `/api/status` reports it.
fn link_name(connected: bool) -> &'static str {
    if connected { "connected" } else { "lost" }
}

fn devices(shared: &Shared) -> Response {
    let devices = shared.devices.lock().unwrap();
    let mapping = shared.mapping.lock().unwrap();
    let values = shared.values.lock().unwrap();
    let outputs = shared.outputs.lock().unwrap();
    let timeline = shared.timeline.lock().unwrap();
    let mut sorted: Vec<&DeviceRec> = devices.iter().collect();
    sorted.sort_by_key(|d| d.id);
    Response::ok(Json::Array(
        sorted
            .into_iter()
            .map(|d| {
                device_json(d, &mapping, &values, outputs.get(&d.id).map(|(o, _)| o))
                    .with_times(timeline.devices.get(&d.id))
            })
            .collect(),
    ))
}

/// One device; with `?menu=`, that menu is discovered first if it has not
/// been, which is how the editor gets at Configuration fields. The device's
/// eventable outputs are asked for the first time it is opened.
fn device(shared: &Shared, serial: &str, menu: Option<&String>) -> Response {
    let id = shared
        .devices
        .lock()
        .unwrap()
        .iter()
        .find(|d| d.serial == serial)
        .map(|d| d.id);
    if let Some(id) = id {
        shared.ensure_outputs(id);
    }
    if let Some(m) = menu {
        let Some(menu) = menu_by_name(m) else {
            return Response::error(400, format!("unknown menu {m:?}"));
        };
        let Some((id, known)) = shared
            .devices
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.serial == serial)
            .map(|d| (d.id, d.menus.contains(&menu)))
        else {
            return Response::error(404, format!("no device with serial {serial}"));
        };
        if !known {
            // Bus access outside the lock: a cold-cache discovery takes a while.
            let groups = match shared.bus.discover_menu(id, menu) {
                Ok(g) => g,
                Err(e) => return Response::error(502, format!("discovering {m}: {e}")),
            };
            let mut devices = shared.devices.lock().unwrap();
            if let Some(d) = devices.iter_mut().find(|d| d.id == id) {
                d.merge_groups(menu, groups);
            }
        }
    }
    let devices = shared.devices.lock().unwrap();
    let mapping = shared.mapping.lock().unwrap();
    let values = shared.values.lock().unwrap();
    let outputs = shared.outputs.lock().unwrap();
    let timeline = shared.timeline.lock().unwrap();
    match devices.iter().find(|d| d.serial == serial) {
        Some(d) => Response::ok(
            device_json(d, &mapping, &values, outputs.get(&d.id).map(|(o, _)| o))
                .with_times(timeline.devices.get(&d.id)),
        ),
        None => Response::error(404, format!("no device with serial {serial}")),
    }
}

fn device_json(
    d: &DeviceRec,
    mapping: &Mapping,
    values: &HashMap<(DeviceId, FieldId), Value>,
    outputs: Option<&Vec<FieldId>>,
) -> Json {
    let dm = mapping.devices.get(&d.serial);
    let fields: Vec<Json> = d
        .fields
        .iter()
        .map(|f| {
            let entry = dm.and_then(|m| m.fields.get(&field_key(f.id)));
            json!({
                "id": field_key(f.id),
                "name": f.name,
                "unit": f.unit,
                "options": f.options,
                "writable": f.writable,
                "menu": menu_name(f.menu),
                "group": f.group,
                "path": entry.map(|e| e.path.clone()),
                "put": entry.is_some_and(|e| e.put),
                "output": outputs.is_some_and(|o| o.contains(&f.id)),
                "eventOnly": outputs.is_some_and(|o| o.contains(&f.id)) && event_only(f),
                "allowOff": entry.is_some_and(|e| e.allow_off),
                "value": values.get(&(d.id, f.id)).map(raw_json),
            })
        })
        .collect();
    json!({
        "id": format!("{:06X}", d.id),
        "serial": d.serial,
        "article": d.article,
        "name": d.name,
        "firmware": d.firmware,
        "instance": dm.map(|m| m.instance.clone()).filter(|i| !i.is_empty()).unwrap_or_else(|| d.instance.clone()),
        "menus": d.menus.iter().map(|m| menu_name(*m)).collect::<Vec<_>>(),
        "mapped": dm.map(|m| m.fields.len()).unwrap_or(0),
        "fields": fields,
    })
}

/// Adds a device's [`DeviceTimes`] to its JSON as `timeline`.
trait WithTimes {
    fn with_times(self, times: Option<&DeviceTimes>) -> Json;
}

impl WithTimes for Json {
    fn with_times(mut self, times: Option<&DeviceTimes>) -> Json {
        self["timeline"] = match times {
            Some(t) => json!({
                "identified": secs(t.identified),
                "took": secs(t.took),
                "late": t.late,
                "streaming": t.streaming.map(secs),
            }),
            None => Json::Null,
        };
        self
    }
}

/// A device value as plain JSON, in the device's own unit: what the editor
/// shows next to a field, and what a write reports back.
pub fn raw_json(v: &Value) -> Json {
    match v {
        // Through the shortest decimal form, so a device's `25.6` does not
        // widen into `25.600000381469727`.
        Value::Float(x) if x.is_finite() => {
            json!(x.to_string().parse::<f64>().unwrap_or(*x as f64))
        }
        Value::Float(_) | Value::Invalid => Json::Null,
        Value::Boolean(b) => json!(b),
        // An option without a label shows as its number, not as "".
        Value::List { index, .. } | Value::Eventable { index, .. } => {
            match v.label().filter(|l| !l.trim().is_empty()) {
                Some(l) => json!(l),
                None => json!(index),
            }
        }
        Value::Text { text, .. } => json!(text),
        Value::Time(t) if t.sec >= 0 => {
            json!(
                t.days as f64 * 86_400.0
                    + t.hour as f64 * 3_600.0
                    + t.min as f64 * 60.0
                    + t.sec as f64
            )
        }
        Value::Time(_) => Json::Null,
        Value::Date(d) if d.day > 0 => json!(format!("{:04}-{:02}-{:02}", d.year, d.mon, d.day)),
        Value::Date(_) => Json::Null,
        Value::DeviceRef { index, .. } => json!(index),
    }
}

/// Look a (serial, field key) pair up; the errors are the responses.
fn locate(
    shared: &Shared,
    serial: &str,
    key: &str,
) -> Result<(DeviceRec, crate::publish::FieldRec), Response> {
    let Some(id) = parse_field_key(key) else {
        return Err(Response::error(400, format!("{key:?} is not a field id")));
    };
    let devices = shared.devices.lock().unwrap();
    let Some(d) = devices.iter().find(|d| d.serial == serial) else {
        return Err(Response::error(
            404,
            format!("no device with serial {serial}"),
        ));
    };
    let Some(f) = d.field(id) else {
        return Err(Response::error(
            404,
            format!(
                "{} has no field {key} (discovered menus: {})",
                d.name,
                d.menus
                    .iter()
                    .map(|m| menu_name(*m))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    };
    Ok((d.clone(), f.clone()))
}

fn read_field(shared: &Shared, serial: &str, key: &str) -> Response {
    let (d, f) = match locate(shared, serial, key) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match shared.bus.read(d.id, f.id) {
        Ok(v) => {
            let v = v.with_options(&f.options);
            shared
                .values
                .lock()
                .unwrap()
                .insert((d.id, f.id), v.clone());
            Response::ok(json!({ "value": raw_json(&v) }))
        }
        Err(e) => Response::error(502, format!("reading {}: {e}", f.name)),
    }
}

#[derive(Deserialize)]
struct WriteBody {
    value: Json,
    #[serde(default)]
    login: Option<LoginBody>,
}

#[derive(Deserialize)]
struct LoginBody {
    level: String,
    code: f32,
}

fn parse_level(s: &str) -> Option<AccessLevel> {
    Some(
        match s
            .trim()
            .to_ascii_lowercase()
            .replace(['_', '-', ' '], "")
            .as_str()
        {
            "enduser" | "user" => AccessLevel::EndUser,
            "installer" => AccessLevel::Installer,
            "distributor" => AccessLevel::Distributor,
            "mvservice" | "service" => AccessLevel::MvService,
            _ => return None,
        },
    )
}

/// A Signal K PUT, landing on the bus. The value arrives in the unit the
/// path publishes (SI for a mapped field, the device's own otherwise) and is
/// converted back with the mapping's plan. A field that is read-only at the
/// current access level is retried once after the supplied login.
fn write_field(shared: &Shared, serial: &str, key: &str, body: &[u8]) -> Response {
    let body: WriteBody = match serde_json::from_slice(body) {
        Ok(b) => b,
        Err(e) => return Response::error(400, format!("body: {e}")),
    };
    let (d, f) = match locate(shared, serial, key) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let entry = shared.mapping.lock().unwrap().field(serial, f.id).cloned();
    let (conv, invert, truth) = entry
        .as_ref()
        .and_then(|e| signalk::plan(&e.path, &f.unit, &f.options, e).ok())
        .map(|p| (p.conv, p.invert, p.truth))
        .unwrap_or((Conversion::IDENTITY, false, BTreeMap::new()));
    let template = match shared.bus.read(d.id, f.id) {
        Ok(v) => v.with_options(&f.options),
        Err(e) => {
            return Response::error(502, format!("reading {} before writing it: {e}", f.name));
        }
    };
    let value = match signalk::decode(&body.value, &template, conv, invert, &truth) {
        Ok(v) => v,
        Err(e) => return Response::error(400, e),
    };
    // An output that events drive is switched the way an event switches
    // it: on always, off only when the mapping allows it (#29).
    if shared.event_only(d.id, &f) {
        let Some(on) = switched_on(&value) else {
            return Response::error(400, format!("{} takes on or off", f.name));
        };
        if !on && !entry.as_ref().is_some_and(|e| e.allow_off) {
            return Response {
                status: 403,
                body: json!({
                    "error": format!(
                        "{} can only be switched on from Signal K; allow switching it off in the mapping",
                        f.name
                    ),
                    "needs": "allowOff",
                }),
            };
        }
        return match shared.bus.set_output(d.id, f.id, on) {
            Ok(applied) => written(shared, &d, &f, applied, conv, invert, &truth),
            Err(e) => Response::error(502, format!("switching {}: {e}", f.name)),
        };
    }
    let mut result = shared.bus.write(d.id, f.id, value.clone());
    if matches!(result, Err(Error::ReadOnly))
        && let Some(login) = &body.login
    {
        let Some(level) = parse_level(&login.level) else {
            return Response::error(400, format!("unknown access level {:?}", login.level));
        };
        match shared.bus.login(d.id, level, login.code) {
            Ok(got) if got == level => result = shared.bus.write(d.id, f.id, value.clone()),
            Ok(got) => {
                return Response::error(
                    403,
                    format!(
                        "{} rejected the {:?} code (still at {got:?})",
                        d.name, level
                    ),
                );
            }
            Err(e) => return Response::error(502, format!("logging in to {}: {e}", d.name)),
        }
    }
    match result {
        Ok(applied) => written(shared, &d, &f, applied, conv, invert, &truth),
        Err(Error::ReadOnly) => Response {
            status: 403,
            body: json!({
                "error": format!("{} is read-only at the current access level", f.name),
                "needs": "login",
            }),
        },
        Err(e @ (Error::WrongType { .. } | Error::InvalidText { .. })) => {
            Response::error(400, e.to_string())
        }
        Err(e) => Response::error(502, format!("writing {}: {e}", f.name)),
    }
}

/// Whether an eventable output is one that a PUT must switch the way an
/// event does: on always, off only when the mapping allows it (#29). That is
/// an output the device does not let us write, or one whose options have no
/// labels: an APR's outputs are writable, but as enums whose labels are all
/// empty, and the batteries drive `Stop charge` for their own protection.
/// Any other writable output (a DSD channel, a CZone control, an INT's
/// `Override`) is a switch people use, written like any other field.
fn event_only(f: &crate::publish::FieldRec) -> bool {
    !f.writable || signalk::unlabeled(&f.options)
}

/// Whether a decoded PUT value means on: a boolean, or a number or list
/// index other than zero.
fn switched_on(v: &Value) -> Option<bool> {
    match v {
        Value::Boolean(b) => Some(*b),
        Value::Float(x) => Some(*x != 0.0),
        Value::List { index, .. } | Value::Eventable { index, .. } => Some(*index != 0),
        _ => None,
    }
}

/// The answer to a write that went through: the value the device shows
/// now, raw and as the path publishes it.
fn written(
    shared: &Shared,
    d: &DeviceRec,
    f: &crate::publish::FieldRec,
    applied: Value,
    conv: Conversion,
    invert: bool,
    truth: &BTreeMap<String, bool>,
) -> Response {
    let applied = applied.with_options(&f.options);
    shared
        .values
        .lock()
        .unwrap()
        .insert((d.id, f.id), applied.clone());
    Response::ok(json!({
        "applied": raw_json(&applied),
        "published": signalk::encode(&applied, conv, invert, truth),
    }))
}

fn put_mapping(shared: &Shared, body: &[u8]) -> Response {
    let m: Mapping = match serde_json::from_slice(body) {
        Ok(m) => m,
        Err(e) => return Response::error(400, format!("mapping: {e}")),
    };
    if m.version != crate::mapping::VERSION {
        return Response::error(
            400,
            format!(
                "mapping version {} is not the {} this build writes",
                m.version,
                crate::mapping::VERSION
            ),
        );
    }
    let mapped = m.len();
    match shared.adopt(m) {
        Ok(r) => Response::ok(json!({
            "mapped": mapped,
            "streaming": r.emit.len(),
            "diagnostics": r.diagnostics,
        })),
        Err(e) => Response::error(500, e),
    }
}

#[derive(Deserialize)]
struct FieldRef {
    serial: String,
    field: String,
}

fn suggest(shared: &Shared, body: &[u8]) -> Response {
    let r: FieldRef = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return Response::error(400, format!("body: {e}")),
    };
    let (d, f) = match locate(shared, &r.serial, &r.field) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let mapping = shared.mapping.lock().unwrap();
    let dm = mapping.devices.get(&d.serial);
    let instance = dm
        .map(|m| m.instance.clone())
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| d.instance.clone());
    let (path, invert, tier) = match dm.and_then(|m| m.fields.get(&field_key(f.id))) {
        Some(e) => (e.path.clone(), e.invert, Some("existing")),
        None => match seed::suggest_or_build(
            &d.article,
            &d.firmware,
            seed::class_of(&d.name),
            &instance,
            publish::mapped_node(dm).as_deref(),
            &seed::FieldCtx {
                id: f.id,
                name: &f.name,
                unit: &f.unit,
                group: &f.group,
            },
        ) {
            Some((s, tier)) => (
                // Not onto a path another field of this device already has.
                seed::unique(
                    s.path,
                    f.id,
                    dm.into_iter().flat_map(|m| {
                        m.fields.iter().filter_map(|(k, e)| {
                            crate::mapping::parse_field_key(k).map(|id| (id, e.path.as_str()))
                        })
                    }),
                ),
                s.invert,
                Some(match tier {
                    seed::Tier::ModelFirmware => "modelFirmware",
                    seed::Tier::Model => "model",
                    seed::Tier::Name => "name",
                    seed::Tier::Built => "built",
                }),
            ),
            None => (publish::path_prefix(dm), false, None),
        },
    };
    Response::ok(json!({
        "path": path,
        "invert": invert,
        "tier": tier,
        "truthDefault": if f.options.is_empty() { None } else { signalk::truth_default(&f.options) },
        "notifyDefault": signalk::notify_default(&f.options),
    }))
}

#[derive(Deserialize)]
struct ValidateBody {
    serial: String,
    field: String,
    entry: FieldMapping,
}

fn validate(shared: &Shared, body: &[u8]) -> Response {
    let b: ValidateBody = match serde_json::from_slice(body) {
        Ok(b) => b,
        Err(e) => return Response::error(400, format!("body: {e}")),
    };
    let (d, f) = match locate(shared, &b.serial, &b.field) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let f_device = d.id;
    let path = b.entry.path.trim();
    if path.is_empty() {
        return Response::ok(
            json!({ "ok": false, "refusal": { "kind": "empty", "message": "no path" } }),
        );
    }
    match signalk::plan(path, &f.unit, &f.options, &b.entry) {
        Ok(p) => {
            let mut warnings: Vec<String> = p.warning.iter().cloned().collect();
            if f.options.len() > 2 && signalk::leaf_is_boolean(path) {
                warnings.push(format!(
                    "{} labels onto a boolean leaf loses information; {} (a string) keeps them all",
                    f.options.len(),
                    signalk::mode_leaf(path)
                ));
            }
            let output = shared.event_only(f_device, &f);
            if b.entry.put && output && !b.entry.allow_off {
                warnings.push(format!(
                    "{} is an output the device's own events also drive; a PUT can only switch it on",
                    f.name
                ));
            }
            if b.entry.put && !f.writable && !output {
                warnings.push("the field is read-only at the current access level; a PUT needs an installer login".into());
            }
            Response::ok(json!({
                "ok": true,
                "unit": p.unit,
                "conversion": p.conv.describe(),
                "boolean": signalk::leaf_is_boolean(path),
                "truth": p.truth,
                "notify": p.notify,
                "invert": p.invert,
                "warnings": warnings,
            }))
        }
        Err(e) => {
            let (kind, labels) = match &e {
                signalk::Refusal::Path { .. } => ("path", Vec::new()),
                signalk::Refusal::Units { .. } => ("units", Vec::new()),
                signalk::Refusal::Truth { labels } => ("truth", labels.clone()),
            };
            // For a truth refusal, what the conventional label meanings can
            // already say, so the editor pre-fills those and asks for the rest.
            let partial: BTreeMap<&str, bool> = labels
                .iter()
                .filter_map(|l| signalk::truth_of_label(l).map(|b| (l.as_str(), b)))
                .collect();
            let hint = match &e {
                signalk::Refusal::Truth { .. } if f.options.len() > 2 => Some(format!(
                    "{} labels onto a boolean leaf loses information; {} (a string) keeps them all",
                    f.options.len(),
                    signalk::mode_leaf(path)
                )),
                _ => None,
            };
            Response::ok(json!({
                "ok": false,
                "refusal": { "kind": kind, "message": e.to_string(), "labels": labels,
                             "truthPartial": partial, "hint": hint },
            }))
        }
    }
}

#[derive(Deserialize)]
struct SerialRef {
    serial: String,
}

/// Copy one device's mapping onto every other device of the same article.
fn apply_article(shared: &Shared, body: &[u8]) -> Response {
    let r: SerialRef = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return Response::error(400, format!("body: {e}")),
    };
    let mut m = shared.mapping.lock().unwrap().clone();
    let Some(src) = m
        .devices
        .get(&r.serial)
        .cloned()
        .filter(|d| !d.fields.is_empty())
    else {
        return Response::error(400, "map at least one field on this device first");
    };
    if src.article.is_empty() {
        return Response::error(400, "this device reports no article number to match on");
    }
    let targets: Vec<CopyTarget> = shared
        .devices
        .lock()
        .unwrap()
        .iter()
        .filter(|d| d.article == src.article && d.serial != r.serial && !d.serial.is_empty())
        .map(|d| CopyTarget {
            instance: d.instance.clone(),
            have: d.field_ids(),
            ident: d.identity(),
        })
        .collect();
    if targets.is_empty() {
        return Response::ok(json!({ "targets": 0, "copied": 0, "skipped": 0, "diagnostics": [] }));
    }
    let (copied, skipped) = copy_to_targets(&mut m, &src, &targets);
    match shared.adopt(m) {
        Ok(res) => Response::ok(json!({
            "targets": targets.len(),
            "copied": copied,
            "skipped": skipped,
            "diagnostics": res.diagnostics,
        })),
        Err(e) => Response::error(500, e),
    }
}

/// Serve requests until the server is unblocked or dropped. Each request is
/// answered on its own thread, because a handler may wait on the bus.
pub fn serve(server: Arc<tiny_http::Server>, shared: Arc<Shared>) {
    for req in server.incoming_requests() {
        let shared = shared.clone();
        std::thread::spawn(move || respond(req, &shared));
    }
}

fn respond(mut req: tiny_http::Request, shared: &Shared) {
    let mut body = Vec::new();
    let _ = req.as_reader().read_to_end(&mut body);
    let mut r = Request::new(req.method().as_str(), req.url(), &body);
    r.bearer = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| {
            h.value
                .as_str()
                .strip_prefix("Bearer ")
                .map(|t| t.trim().to_string())
        });
    let resp = handle(shared, &r);
    let json = serde_json::to_string(&resp.body).unwrap_or_else(|_| "null".into());
    let out = tiny_http::Response::from_string(json)
        .with_status_code(resp.status)
        .with_header(
            tiny_http::Header::from_bytes("Content-Type", "application/json")
                .expect("static header"),
        );
    let _ = req.respond(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::DeviceMapping;
    use crate::publish::tests::{dev, mli};
    use crossbeam_channel::{Receiver, unbounded};
    use std::sync::Mutex;

    /// A bus made of tables: values per field, which fields are writable,
    /// and the login code each device accepts. The state is shared with the
    /// test through an `Arc`, since the bus itself is boxed into `Shared`.
    struct StubBus(Arc<StubState>);

    struct StubState {
        values: Mutex<HashMap<(DeviceId, FieldId), Value>>,
        writable: Mutex<HashMap<(DeviceId, FieldId), bool>>,
        code: f32,
        level: Mutex<AccessLevel>,
        menus: Mutex<HashMap<(DeviceId, Menu), Vec<GroupInfo>>>,
        writes: Mutex<Vec<(DeviceId, FieldId, Value)>>,
        discoveries: Mutex<usize>,
        /// Fields that answer when asked for on their own.
        fields: Mutex<HashMap<(DeviceId, FieldId), FieldInfo>>,
        field_asks: Mutex<usize>,
        /// Each device's eventable outputs.
        outputs: Mutex<HashMap<DeviceId, Vec<FieldId>>>,
        /// Outputs switched, in order.
        switched: Mutex<Vec<(DeviceId, FieldId, bool)>>,
        output_asks: Mutex<usize>,
    }

    impl StubBus {
        fn new(code: f32) -> (StubBus, Arc<StubState>) {
            let state = Arc::new(StubState {
                values: Mutex::new(HashMap::new()),
                writable: Mutex::new(HashMap::new()),
                code,
                level: Mutex::new(AccessLevel::EndUser),
                menus: Mutex::new(HashMap::new()),
                writes: Mutex::new(Vec::new()),
                discoveries: Mutex::new(0),
                fields: Mutex::new(HashMap::new()),
                field_asks: Mutex::new(0),
                outputs: Mutex::new(HashMap::new()),
                switched: Mutex::new(Vec::new()),
                output_asks: Mutex::new(0),
            });
            (StubBus(state.clone()), state)
        }
        fn plain() -> StubBus {
            StubBus::new(0.0).0
        }
    }

    impl BusOps for StubBus {
        fn discover_menu(&self, id: DeviceId, menu: Menu) -> masterbus::Result<Vec<GroupInfo>> {
            *self.0.discoveries.lock().unwrap() += 1;
            self.0
                .menus
                .lock()
                .unwrap()
                .get(&(id, menu))
                .cloned()
                .ok_or(Error::Timeout)
        }
        fn field_info(&self, id: DeviceId, field: FieldId) -> masterbus::Result<Option<FieldInfo>> {
            *self.0.field_asks.lock().unwrap() += 1;
            Ok(self.0.fields.lock().unwrap().get(&(id, field)).cloned())
        }
        fn read(&self, id: DeviceId, field: FieldId) -> masterbus::Result<Value> {
            self.0
                .values
                .lock()
                .unwrap()
                .get(&(id, field))
                .cloned()
                .ok_or(Error::NotReady)
        }
        fn write(&self, id: DeviceId, field: FieldId, value: Value) -> masterbus::Result<Value> {
            let writable = *self
                .0
                .writable
                .lock()
                .unwrap()
                .get(&(id, field))
                .unwrap_or(&false)
                || *self.0.level.lock().unwrap() != AccessLevel::EndUser;
            if !writable {
                return Err(Error::ReadOnly);
            }
            self.0
                .writes
                .lock()
                .unwrap()
                .push((id, field, value.clone()));
            self.0
                .values
                .lock()
                .unwrap()
                .insert((id, field), value.clone());
            Ok(value)
        }
        fn login(
            &self,
            _id: DeviceId,
            level: AccessLevel,
            code: f32,
        ) -> masterbus::Result<AccessLevel> {
            if code == self.0.code {
                *self.0.level.lock().unwrap() = level;
            }
            Ok(*self.0.level.lock().unwrap())
        }
        fn eventable_outputs(&self, id: DeviceId) -> masterbus::Result<Vec<FieldId>> {
            *self.0.output_asks.lock().unwrap() += 1;
            Ok(self
                .0
                .outputs
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_default())
        }
        fn set_output(&self, id: DeviceId, field: FieldId, on: bool) -> masterbus::Result<Value> {
            self.0.switched.lock().unwrap().push((id, field, on));
            let mut values = self.0.values.lock().unwrap();
            // The field keeps its kind; an enum selects option 1 or 0.
            let v = match values.get(&(id, field)) {
                Some(Value::List { options, .. }) => Value::List {
                    index: i32::from(on),
                    options: options.clone(),
                },
                _ => Value::Float(if on { 1.0 } else { 0.0 }),
            };
            values.insert((id, field), v.clone());
            Ok(v)
        }
    }

    fn shared_with(
        bus: StubBus,
        devices: Vec<DeviceRec>,
        mapping: Mapping,
    ) -> (Arc<Shared>, Receiver<()>) {
        let (tx, rx) = unbounded();
        let shared = Shared {
            bus: Box::new(bus),
            devices: Mutex::new(devices),
            mapping: Mutex::new(mapping),
            diagnostics: Mutex::new(Vec::new()),
            values: Mutex::new(HashMap::new()),
            mapping_path: None,
            reload: tx,
            started: Instant::now(),
            info: Info {
                transport: "can:vcan0".into(),
                stream: "0.0.0.0:3009".into(),
                api: "127.0.0.1:3010".into(),
            },
            token: None,
            streaming: AtomicUsize::new(0),
            clients: AtomicUsize::new(0),
            asked: Mutex::new(HashSet::new()),
            outputs: Mutex::new(HashMap::new()),
            timeline: Mutex::new(Timeline::default()),
        };
        (Arc::new(shared), rx)
    }

    fn get(shared: &Shared, url: &str) -> Response {
        handle(shared, &Request::new("GET", url, b""))
    }
    fn send(shared: &Shared, method: &str, url: &str, body: Json) -> Response {
        handle(
            shared,
            &Request::new(method, url, body.to_string().as_bytes()),
        )
    }

    #[test]
    fn a_token_is_required_when_configured() {
        let (shared, _) = shared_with(StubBus::plain(), vec![], Mapping::new());
        let mut s = Arc::try_unwrap(shared).ok().unwrap();
        s.token = Some("hunter2".into());
        assert_eq!(get(&s, "/api/status").status, 401);
        assert_eq!(
            handle(&s, &Request::new("GET", "/api/status", b"").bearer("wrong")).status,
            401
        );
        assert_eq!(
            handle(
                &s,
                &Request::new("GET", "/api/status", b"").bearer("hunter2")
            )
            .status,
            200
        );
        // Without a configured token, anything goes (the daemon only allows
        // that on loopback).
        s.token = None;
        assert_eq!(get(&s, "/api/status").status, 200);
        assert_eq!(get(&s, "/nope").status, 404);
        assert_eq!(get(&s, "/api/nope").status, 404);
    }

    #[test]
    fn status_counts_what_the_daemon_knows() {
        let devices = vec![mli()];
        let mapping = publish::seed_mapping(&devices);
        let (shared, _) = shared_with(StubBus::plain(), devices, mapping.clone());
        shared.streaming.store(3, Ordering::Relaxed);
        let r = get(&shared, "/api/status");
        assert_eq!(r.status, 200);
        assert_eq!(r.body["apiVersion"], API_VERSION);
        assert_eq!(r.body["devices"], 1);
        assert_eq!(r.body["mapped"]["fields"], mapping.len());
        assert_eq!(r.body["streaming"], 3);
        assert_eq!(r.body["transport"], "can:vcan0");
        assert_eq!(r.body["link"], "connected");
        assert_eq!(link_name(false), "lost");
        assert_eq!(r.body["walked"], Json::Null);
        shared.timeline.lock().unwrap().walked = Some(Duration::from_millis(171_240));
        assert_eq!(get(&shared, "/api/status").body["walked"], 171.2);
    }

    /// Each device says when it was identified and when its fields started
    /// streaming, so a slow start can be measured from the API (#40).
    #[test]
    fn devices_carry_when_they_were_identified_and_started_streaming() {
        let devices = vec![mli()];
        let id = devices[0].id;
        let mapping = publish::seed_mapping(&devices);
        let (shared, _) = shared_with(StubBus::plain(), devices, mapping);
        // Not identified by the daemon's discovery (a test or a stub): null.
        assert_eq!(get(&shared, "/api/devices").body[0]["timeline"], Json::Null);
        shared.timeline.lock().unwrap().devices.insert(
            id,
            DeviceTimes {
                identified: Duration::from_millis(312_440),
                took: Duration::from_millis(8_050),
                late: true,
                streaming: None,
            },
        );
        let t = &get(&shared, "/api/devices").body[0]["timeline"];
        assert_eq!(t["identified"], 312.4);
        assert_eq!(t["took"], 8.1);
        assert_eq!(t["late"], true);
        assert_eq!(t["streaming"], Json::Null);
        shared
            .timeline
            .lock()
            .unwrap()
            .devices
            .get_mut(&id)
            .unwrap()
            .streaming = Some(Duration::from_millis(313_000));
        let d = get(&shared, "/api/devices/MLI-1").body;
        assert_eq!(d["timeline"]["streaming"], 313.0);
    }

    /// The device listing carries everything the editor shows per field: the
    /// mapped path, the put flag, and the last value the stream saw, in the
    /// device's own unit.
    #[test]
    fn devices_list_fields_with_their_mapping_and_last_value() {
        let devices = vec![mli()];
        let id = devices[0].id;
        let mapping = publish::seed_mapping(&devices);
        let (shared, _) = shared_with(StubBus::plain(), devices, mapping);
        shared
            .values
            .lock()
            .unwrap()
            .insert((id, 0x005), Value::Float(20.0));
        let r = get(&shared, "/api/devices");
        assert_eq!(r.status, 200);
        let d = &r.body[0];
        assert_eq!(d["serial"], "MLI-1");
        assert_eq!(d["id"], "100005");
        assert_eq!(d["instance"], "24vService");
        assert_eq!(d["menus"], json!(["monitoring"]));
        let temp = d["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == "0x005")
            .unwrap();
        assert_eq!(temp["path"], "electrical.batteries.24vService.temperature");
        assert_eq!(temp["value"], 20.0);
        assert_eq!(temp["put"], false);
        assert_eq!(temp["menu"], "monitoring");
        let relay = d["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == "0x022")
            .unwrap();
        assert_eq!(relay["path"], Json::Null);
        assert_eq!(relay["value"], Json::Null);
        assert_eq!(get(&shared, "/api/devices/NOPE").status, 404);
    }

    /// `?menu=configuration` discovers the menu once and merges its fields;
    /// asking again does not hit the bus.
    #[test]
    fn asking_for_a_menu_discovers_it_once() {
        let devices = vec![mli()];
        let id = devices[0].id;
        let (bus, state) = StubBus::new(0.0);
        state.menus.lock().unwrap().insert(
            (id, Menu::Configuration),
            vec![GroupInfo {
                id: 9,
                name: "Inverter".into(),
                menu: Menu::Configuration,
                fields: vec![masterbus::FieldInfo {
                    index: 0x013,
                    name: "Inverter".into(),
                    unit: String::new(),
                    viz_type: masterbus::VisualizationType::CheckBox,
                    writeable: true,
                    eventable: false,
                    min: 0.0,
                    max: 1.0,
                    step: 1.0,
                    options: vec![],
                }],
            }],
        );
        let (shared, _) = shared_with(bus, devices, Mapping::new());
        let r = get(&shared, "/api/devices/MLI-1?menu=configuration");
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["menus"], json!(["monitoring", "configuration"]));
        let f = r.body["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == "0x013")
            .unwrap();
        assert_eq!(f["writable"], true);
        assert_eq!(f["group"], "Inverter");
        // Now known: asking again does not go to the bus.
        assert_eq!(*state.discoveries.lock().unwrap(), 1);
        assert_eq!(
            get(&shared, "/api/devices/MLI-1?menu=configuration").status,
            200
        );
        assert_eq!(*state.discoveries.lock().unwrap(), 1);
        // A menu the bus cannot deliver is a bus error; a made-up one a client error.
        assert_eq!(get(&shared, "/api/devices/MLI-1?menu=service").status, 502);
        assert_eq!(get(&shared, "/api/devices/MLI-1?menu=banana").status, 400);
    }

    #[test]
    fn reading_a_field_polls_and_caches() {
        let devices = vec![mli()];
        let id = devices[0].id;
        let (bus, state) = StubBus::new(0.0);
        state
            .values
            .lock()
            .unwrap()
            .insert((id, 0x001), Value::Float(25.6));
        let (shared, _) = shared_with(bus, devices, Mapping::new());
        let r = get(&shared, "/api/devices/MLI-1/fields/0x001/value");
        assert_eq!(r.status, 200);
        assert_eq!(r.body["value"], 25.6);
        assert_eq!(
            shared.values.lock().unwrap()[&(id, 0x001)],
            Value::Float(25.6)
        );
        assert_eq!(
            get(&shared, "/api/devices/MLI-1/fields/0x0FF/value").status,
            404
        );
        assert_eq!(
            get(&shared, "/api/devices/MLI-1/fields/zz/value").status,
            400
        );
        // A field the stub has no value for is a bus error, not a crash.
        assert_eq!(
            get(&shared, "/api/devices/MLI-1/fields/0x002/value").status,
            502
        );
    }

    /// A mapping naming a field outside Monitoring (a PUT target on
    /// Configuration) has that menu discovered before it is resolved, so
    /// the answer does not claim "no such field" for a field the device has.
    #[test]
    fn adopting_a_mapping_discovers_the_menu_a_put_target_lives_on() {
        let devices = vec![mli()];
        let id = devices[0].id;
        let (bus, state) = StubBus::new(0.0);
        state.menus.lock().unwrap().insert(
            (id, Menu::Configuration),
            vec![GroupInfo {
                id: 9,
                name: "Relay".into(),
                menu: Menu::Configuration,
                fields: vec![masterbus::FieldInfo {
                    index: 0x040,
                    name: "Relay".into(),
                    unit: String::new(),
                    viz_type: masterbus::VisualizationType::CheckBox,
                    writeable: true,
                    eventable: false,
                    min: 0.0,
                    max: 1.0,
                    step: 1.0,
                    options: vec![],
                }],
            }],
        );
        let (shared, _) = shared_with(bus, devices, Mapping::new());
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x040),
            FieldMapping {
                path: "electrical.switches.relay.state".into(),
                put: true,
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        let r = send(&shared, "PUT", "/api/mapping", json!(m));
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["streaming"], 1);
        assert_eq!(r.body["diagnostics"], json!([]));
        assert_eq!(*state.discoveries.lock().unwrap(), 1);
        let d = get(&shared, "/api/devices/MLI-1").body;
        assert_eq!(d["menus"], json!(["monitoring", "configuration"]));
        // Adopting again does not discover again.
        send(&shared, "PUT", "/api/mapping", json!(m));
        assert_eq!(*state.discoveries.lock().unwrap(), 1);
    }

    /// A mapped field that neither Monitoring nor Configuration lists (one
    /// shown only at installer level, mapped in the TUI after a login) is
    /// asked for on its own and published; one the device doesn't have is
    /// reported, and neither is asked for twice.
    #[test]
    fn a_mapped_field_no_menu_lists_is_asked_for_on_its_own() {
        let devices = vec![mli()];
        let id = devices[0].id;
        let (bus, state) = StubBus::new(0.0);
        state.fields.lock().unwrap().insert(
            (id, 0x086),
            FieldInfo {
                index: 0x086,
                name: "Avg. cell high".into(),
                unit: "V".into(),
                viz_type: masterbus::VisualizationType::Float,
                writeable: false,
                eventable: false,
                min: 0.0,
                max: 5.0,
                step: 0.01,
                options: vec![],
            },
        );
        let (shared, _) = shared_with(bus, devices, Mapping::new());
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        for (f, path) in [
            (0x086, "electrical.batteries.service.cellVoltageHigh"),
            (0x0F0, "electrical.batteries.service.nothing"),
        ] {
            dm.fields.insert(
                field_key(f),
                FieldMapping {
                    path: path.into(),
                    ..Default::default()
                },
            );
        }
        m.devices.insert("MLI-1".into(), dm);
        let r = send(&shared, "PUT", "/api/mapping", json!(m));
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["streaming"], 1);
        let diags = r.body["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0]["field"], "0x0F0");
        assert_eq!(*state.field_asks.lock().unwrap(), 2);
        send(&shared, "PUT", "/api/mapping", json!(m));
        assert_eq!(*state.field_asks.lock().unwrap(), 2);
    }

    /// Adopting a mapping over the API persists it, re-resolves it against
    /// the devices, answers with the diagnostics and wakes the daemon.
    #[test]
    fn putting_a_mapping_persists_resolves_and_signals_reload() {
        let dir = std::env::temp_dir().join(format!("mb-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mapping.json");
        let devices = vec![mli()];
        let (shared, rx) = shared_with(StubBus::plain(), devices, Mapping::new());
        let mut s = Arc::try_unwrap(shared).ok().unwrap();
        s.mapping_path = Some(path.clone());

        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x001),
            FieldMapping {
                path: "electrical.batteries.house.voltage".into(),
                ..Default::default()
            },
        );
        dm.fields.insert(
            field_key(0x002),
            FieldMapping {
                path: "electrical.batteries.house.temperature".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        let r = send(&s, "PUT", "/api/mapping", json!(m));
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["mapped"], 2);
        assert_eq!(r.body["streaming"], 1);
        let diags = r.body["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0]["severity"], "error");
        assert_eq!(diags[0]["field"], "0x002");
        assert!(rx.try_recv().is_ok(), "the daemon is told to re-activate");
        assert_eq!(Mapping::load(&path).unwrap(), m);
        assert_eq!(*s.mapping.lock().unwrap(), m);
        assert_eq!(get(&s, "/api/mapping").body, json!(m));
        assert_eq!(
            get(&s, "/api/mapping/diagnostics")
                .body
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // Garbage and a foreign version are refused without touching anything.
        assert_eq!(
            handle(&s, &Request::new("PUT", "/api/mapping", b"{ nope")).status,
            400
        );
        let mut v2 = m.clone();
        v2.version = 2;
        assert_eq!(send(&s, "PUT", "/api/mapping", json!(v2)).status, 400);
        assert_eq!(*s.mapping.lock().unwrap(), m);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_proposes_from_the_tables_and_falls_back_to_a_prefix() {
        let mut d = mli();
        d.fields.push(crate::publish::FieldRec {
            id: 0x030,
            name: "Something odd".into(),
            unit: "V".into(),
            options: vec![],
            writable: false,
            menu: Menu::Monitoring,
            group: "Misc".into(),
        });
        let (shared, _) = shared_with(StubBus::plain(), vec![d], Mapping::new());
        let r = send(
            &shared,
            "POST",
            "/api/mapping/suggest",
            json!({"serial": "MLI-1", "field": "0x001"}),
        );
        assert_eq!(r.status, 200);
        assert_eq!(r.body["path"], "electrical.batteries.24vService.voltage");
        assert_eq!(r.body["tier"], "name");
        // No rule knows it: a path built from its group, name and unit.
        let r = send(
            &shared,
            "POST",
            "/api/mapping/suggest",
            json!({"serial": "MLI-1", "field": "0x030"}),
        );
        assert_eq!(
            r.body["path"],
            "electrical.batteries.24vService.misc.somethingOdd.voltage"
        );
        assert_eq!(r.body["tier"], "built");
        // Once something is mapped, a built path joins it; and an existing
        // entry comes back as itself.
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x001),
            FieldMapping {
                path: "electrical.batteries.house.voltage".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        *shared.mapping.lock().unwrap() = m;
        let r = send(
            &shared,
            "POST",
            "/api/mapping/suggest",
            json!({"serial": "MLI-1", "field": "0x030"}),
        );
        assert_eq!(
            r.body["path"],
            "electrical.batteries.house.misc.somethingOdd.voltage"
        );
        let r = send(
            &shared,
            "POST",
            "/api/mapping/suggest",
            json!({"serial": "MLI-1", "field": "0x001"}),
        );
        assert_eq!(r.body["path"], "electrical.batteries.house.voltage");
        assert_eq!(r.body["tier"], "existing");
    }

    #[test]
    fn suggest_prefills_truth_and_notify_tables_for_enums() {
        let mut d = mli();
        d.fields.push(crate::publish::FieldRec {
            id: 0x010,
            name: "Device state".into(),
            unit: String::new(),
            options: vec!["Standby".into(), "On".into(), "Alarm".into()],
            writable: false,
            menu: Menu::Monitoring,
            group: "State".into(),
        });
        let (shared, _) = shared_with(StubBus::plain(), vec![d], Mapping::new());
        let r = send(
            &shared,
            "POST",
            "/api/mapping/suggest",
            json!({"serial": "MLI-1", "field": "0x010"}),
        );
        assert_eq!(r.body["truthDefault"], Json::Null, "Alarm is ambiguous");
        assert_eq!(r.body["notifyDefault"], json!({"Alarm": "alarm"}));
    }

    #[test]
    fn validate_reports_the_plan_or_the_refusal() {
        let mut d = mli();
        d.fields.push(crate::publish::FieldRec {
            id: 0x010,
            name: "State".into(),
            unit: String::new(),
            options: vec!["Standby".into(), "On".into(), "Alarm".into()],
            writable: false,
            menu: Menu::Monitoring,
            group: "State".into(),
        });
        let (shared, _) = shared_with(StubBus::plain(), vec![d], Mapping::new());
        let v = |field: &str, entry: Json| {
            send(
                &shared,
                "POST",
                "/api/mapping/validate",
                json!({"serial": "MLI-1", "field": field, "entry": entry}),
            )
        };
        let r = v(
            "0x005",
            json!({"path": "electrical.batteries.house.temperature"}),
        );
        assert_eq!(r.body["ok"], true, "{}", r.body);
        assert_eq!(r.body["unit"], "K");
        assert!(r.body["conversion"].as_str().unwrap().contains("273.15"));
        assert_eq!(r.body["warnings"], json!([]));

        let r = v(
            "0x002",
            json!({"path": "electrical.batteries.house.temperature"}),
        );
        assert_eq!(r.body["ok"], false);
        assert_eq!(r.body["refusal"]["kind"], "units");

        let r = v("0x010", json!({"path": "electrical.switches.x.state"}));
        assert_eq!(r.body["ok"], false);
        assert_eq!(r.body["refusal"]["kind"], "truth");
        assert_eq!(
            r.body["refusal"]["labels"],
            json!(["Standby", "On", "Alarm"])
        );

        // With a table it works, but three labels onto a boolean is warned about.
        let r = v(
            "0x010",
            json!({"path": "electrical.switches.x.state", "truth": {"Standby": false, "On": true, "Alarm": false}}),
        );
        assert_eq!(r.body["ok"], true);
        assert_eq!(r.body["boolean"], true);
        assert!(r.body["warnings"][0].as_str().unwrap().contains("3 labels"));

        // A put on a read-only field is allowed, with a warning.
        let r = v(
            "0x005",
            json!({"path": "electrical.batteries.house.temperature", "put": true}),
        );
        assert_eq!(r.body["ok"], true);
        assert!(
            r.body["warnings"][0]
                .as_str()
                .unwrap()
                .contains("read-only")
        );

        let r = v("0x005", json!({"path": "  "}));
        assert_eq!(r.body["ok"], false);
        assert_eq!(r.body["refusal"]["kind"], "empty");
        // A prefix, as the editor pre-fills it, is not a path.
        let r = v("0x005", json!({"path": "electrical."}));
        assert_eq!(r.body["ok"], false);
        assert_eq!(r.body["refusal"]["kind"], "path");
        assert!(
            r.body["refusal"]["message"]
                .as_str()
                .unwrap()
                .contains("prefix")
        );
        // A truth refusal says what the conventions already know, and steers
        // three labels towards the mode leaf.
        let r = v("0x010", json!({"path": "electrical.inverters.x.enabled"}));
        assert_eq!(r.body["refusal"]["kind"], "truth");
        assert_eq!(
            r.body["refusal"]["truthPartial"],
            json!({"Standby": false, "On": true})
        );
        assert!(
            r.body["refusal"]["hint"]
                .as_str()
                .unwrap()
                .contains("inverterMode")
        );
    }

    #[test]
    fn apply_article_copies_onto_the_other_units() {
        let a = mli();
        let mut b = dev(
            "MLI-2",
            "BAT 24V Service2",
            &[(0x001, "Voltage", "V"), (0x002, "Current", "A")],
        );
        b.id = 0x100099;
        let mut m = Mapping::new();
        let mut dm = DeviceMapping {
            article: "66026000".into(),
            instance: "24vService".into(),
            ..Default::default()
        };
        dm.fields.insert(
            field_key(0x001),
            FieldMapping {
                path: "electrical.batteries.24vService.voltage".into(),
                ..Default::default()
            },
        );
        dm.fields.insert(
            field_key(0x005),
            FieldMapping {
                path: "electrical.batteries.24vService.temperature".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        let (shared, rx) = shared_with(StubBus::plain(), vec![a, b], m);
        let r = send(
            &shared,
            "POST",
            "/api/mapping/apply-article",
            json!({"serial": "MLI-1"}),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["targets"], 1);
        assert_eq!(r.body["copied"], 1);
        assert_eq!(r.body["skipped"], 1, "MLI-2 has no temperature field");
        assert!(rx.try_recv().is_ok());
        let m = shared.mapping.lock().unwrap().clone();
        assert_eq!(
            m.devices["MLI-2"].fields[&field_key(0x001)].path,
            "electrical.batteries.24vService2.voltage"
        );
        // Nothing mapped on the source: a request error, not a copy of nothing.
        assert_eq!(
            send(
                &shared,
                "POST",
                "/api/mapping/apply-article",
                json!({"serial": "MLI-2-nope"})
            )
            .status,
            400
        );
    }

    /// An APR-like device: `Stop charge` (`0x027`) is its fourth eventable
    /// output, which no menu lists. As on Peter's APR (#29) it reports itself
    /// writable, as an enum of three options whose labels are all empty.
    fn apr() -> (Arc<Shared>, Arc<StubState>, DeviceId) {
        let d = dev("APR-1", "APR ALT 110A", &[(0x005, "Charger state", "")]);
        let id = d.id;
        let (bus, state) = StubBus::new(0.0);
        state
            .outputs
            .lock()
            .unwrap()
            .insert(id, [0x021, 0x023, 0x025, 0x027].to_vec());
        state.fields.lock().unwrap().insert(
            (id, 0x027),
            masterbus::FieldInfo {
                index: 0x027,
                name: "Stop charge".into(),
                unit: String::new(),
                viz_type: masterbus::VisualizationType::DropDown,
                writeable: true,
                eventable: true,
                min: 0.0,
                max: 3.0,
                step: 1.0,
                options: vec![String::new(); 3],
            },
        );
        state.writable.lock().unwrap().insert((id, 0x027), true);
        state.values.lock().unwrap().insert(
            (id, 0x027),
            Value::List {
                index: 0,
                options: vec![String::new(); 3],
            },
        );
        let (shared, _) = shared_with(bus, vec![d], Mapping::new());
        (shared, state, id)
    }

    /// Opening a device asks it once for its eventable outputs, and the ones
    /// no menu lists become fields the editor can map.
    #[test]
    fn opening_a_device_adds_the_outputs_no_menu_lists() {
        let (shared, state, _) = apr();
        let r = get(&shared, "/api/devices/APR-1");
        assert_eq!(r.status, 200, "{}", r.body);
        let fields = r.body["fields"].as_array().unwrap();
        let stop = fields
            .iter()
            .find(|f| f["id"] == "0x027")
            .expect("Stop charge");
        assert_eq!(stop["name"], "Stop charge");
        assert_eq!(stop["output"], true);
        assert_eq!(stop["writable"], true);
        assert_eq!(stop["eventOnly"], true, "writable, but an enum");
        let state_field = fields.iter().find(|f| f["id"] == "0x005").unwrap();
        assert_eq!(state_field["output"], false);
        // The three outputs the stub can't describe are left out, and the
        // list is not asked for again.
        assert_eq!(fields.len(), 2);
        assert_eq!(*state.field_asks.lock().unwrap(), 4);
        get(&shared, "/api/devices/APR-1");
        assert_eq!(*state.field_asks.lock().unwrap(), 4);
    }

    /// A PUT switches such an output on; off only when the mapping entry
    /// allows it, since the batteries' own events drive it too.
    #[test]
    fn an_output_is_switched_off_only_when_the_mapping_allows_it() {
        let (shared, state, id) = apr();
        get(&shared, "/api/devices/APR-1");
        let entry = |allow_off| FieldMapping {
            path: "electrical.chargers.alternator.stopCharge".into(),
            put: true,
            allow_off,
            ..Default::default()
        };
        let mut dm = DeviceMapping::default();
        dm.fields.insert(field_key(0x027), entry(false));
        shared
            .mapping
            .lock()
            .unwrap()
            .devices
            .insert("APR-1".into(), dm);
        let put = |v: Json| {
            send(
                &shared,
                "PUT",
                "/api/devices/APR-1/fields/0x027",
                json!({ "value": v }),
            )
        };

        let r = put(json!(true));
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["applied"], 1, "option 1, which has no label");
        assert_eq!(r.body["published"], true);
        let r = put(json!(false));
        assert_eq!(r.status, 403, "{}", r.body);
        assert_eq!(r.body["needs"], "allowOff");
        assert_eq!(*state.switched.lock().unwrap(), vec![(id, 0x027, true)]);
        // Never through the ordinary write, which the device would refuse.
        assert!(state.writes.lock().unwrap().is_empty());

        shared
            .mapping
            .lock()
            .unwrap()
            .devices
            .get_mut("APR-1")
            .unwrap()
            .fields
            .insert(field_key(0x027), entry(true));
        assert_eq!(put(json!(false)).status, 200);
        assert_eq!(
            *state.switched.lock().unwrap(),
            vec![(id, 0x027, true), (id, 0x027, false)]
        );
    }

    /// Every kind of value a PUT can decode to says on or off the same way.
    #[test]
    fn a_put_value_means_on_unless_it_is_zero() {
        assert_eq!(switched_on(&Value::Boolean(true)), Some(true));
        assert_eq!(switched_on(&Value::Float(0.0)), Some(false));
        let on = Value::Eventable {
            index: 1,
            labels: vec!["Off".into(), "On".into()],
        };
        assert_eq!(switched_on(&on), Some(true));
        assert_eq!(switched_on(&Value::Invalid), None);
    }

    /// A device with no outputs is not asked again on every PUT to a
    /// read-only field; its answer is kept for a while.
    #[test]
    fn no_outputs_is_remembered_for_a_while() {
        let mut d = mli();
        d.fields[0].writable = false;
        let id = d.id;
        let (bus, state) = StubBus::new(0.0);
        state
            .values
            .lock()
            .unwrap()
            .insert((id, 0x000), Value::Float(50.0));
        let (shared, _) = shared_with(bus, vec![d], Mapping::new());
        for _ in 0..3 {
            let r = send(
                &shared,
                "PUT",
                "/api/devices/MLI-1/fields/0x000",
                json!({"value": 60}),
            );
            assert_eq!(r.body["needs"], "login", "{}", r.body);
        }
        assert_eq!(*state.output_asks.lock().unwrap(), 1);
        // Once the answer is old, the device is asked again.
        shared.outputs.lock().unwrap().get_mut(&id).unwrap().1 -= NO_OUTPUTS_KEPT;
        get(&shared, "/api/devices/MLI-1");
        assert_eq!(*state.output_asks.lock().unwrap(), 2);
    }

    /// If the device's output list could not be had (a busy bus at the first
    /// ask), an APR's writable, label-less `Stop charge` still never takes the
    /// ordinary write: off stays refused without `allow_off`, and on goes
    /// through the switch, which refuses a field the device does not list.
    #[test]
    fn an_output_without_labels_is_guarded_without_the_output_list() {
        let (shared, state, id) = apr();
        state.outputs.lock().unwrap().remove(&id);
        let d = shared.devices.lock().unwrap()[0].clone();
        shared.devices.lock().unwrap()[0]
            .fields
            .push(crate::publish::FieldRec {
                id: 0x027,
                name: "Stop charge".into(),
                unit: String::new(),
                options: vec![String::new(); 3],
                writable: true,
                menu: Menu::Monitoring,
                group: String::new(),
            });
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x027),
            FieldMapping {
                path: "electrical.chargers.alternator.stopCharge".into(),
                put: true,
                ..Default::default()
            },
        );
        shared
            .mapping
            .lock()
            .unwrap()
            .devices
            .insert(d.serial.clone(), dm);
        let r = send(
            &shared,
            "PUT",
            "/api/devices/APR-1/fields/0x027",
            json!({"value": false}),
        );
        assert_eq!(r.status, 403, "{}", r.body);
        assert_eq!(r.body["needs"], "allowOff");
        assert!(state.writes.lock().unwrap().is_empty());
    }

    /// A DSD channel is an eventable output too, but a writable switch: a
    /// PUT writes it like any other field, off included, with no
    /// `allow_off` needed. Suster Anna switches its DSD channels this way.
    #[test]
    fn a_writable_switch_that_is_also_an_output_switches_off_as_before() {
        let mut d = dev("DSD-1", "DSD 1 Power", &[]);
        d.fields.push(crate::publish::FieldRec {
            id: 0x011,
            name: "Backup H5 Pilot".into(),
            unit: String::new(),
            options: vec![],
            writable: true,
            menu: Menu::Monitoring,
            group: "Outputs".into(),
        });
        let id = d.id;
        let (bus, state) = StubBus::new(0.0);
        state.outputs.lock().unwrap().insert(id, vec![0x011]);
        state.writable.lock().unwrap().insert((id, 0x011), true);
        state
            .values
            .lock()
            .unwrap()
            .insert((id, 0x011), Value::Boolean(true));
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x011),
            FieldMapping {
                path: "electrical.switches.dsd1.port8.state".into(),
                put: true,
                ..Default::default()
            },
        );
        m.devices.insert("DSD-1".into(), dm);
        let (shared, _) = shared_with(bus, vec![d], m);

        let dev_json = get(&shared, "/api/devices/DSD-1").body;
        assert_eq!(dev_json["fields"][0]["output"], true);
        assert_eq!(dev_json["fields"][0]["eventOnly"], false);
        let r = send(
            &shared,
            "PUT",
            "/api/devices/DSD-1/fields/0x011",
            json!({"value": false}),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(
            *state.writes.lock().unwrap(),
            vec![(id, 0x011, Value::Boolean(false))]
        );
        assert!(state.switched.lock().unwrap().is_empty());
    }

    /// Only an output that cannot be written, or whose options have no
    /// labels, is switched like an event. A labelled writable list, like an
    /// INT's `Override` (`Standby` / `Activated`), stays an ordinary switch
    /// even if a device lists it among its outputs.
    #[test]
    fn only_unwritable_or_unlabeled_outputs_are_event_only() {
        let f = |writable, options: &[&str]| crate::publish::FieldRec {
            id: 0x001,
            name: "x".into(),
            unit: String::new(),
            options: options.iter().map(|s| s.to_string()).collect(),
            writable,
            menu: Menu::Monitoring,
            group: String::new(),
        };
        assert!(event_only(&f(false, &[])));
        assert!(event_only(&f(true, &["", "", ""])));
        assert!(!event_only(&f(true, &[])));
        assert!(!event_only(&f(true, &["Standby", "Activated"])));
    }

    /// An option without a label reads as its number, not as "".
    #[test]
    fn an_option_without_a_label_shows_its_number() {
        let v = Value::List {
            index: 2,
            options: vec![String::new(); 3],
        };
        assert_eq!(raw_json(&v), json!(2));
    }

    /// A Signal K PUT of `true` on an inverted boolean writes `false` to the
    /// device; the answer says both what was applied and what will publish.
    #[test]
    fn writing_a_field_converts_through_the_mapping() {
        let mut d = mli();
        d.fields.push(crate::publish::FieldRec {
            id: 0x015,
            name: "Standby".into(),
            unit: String::new(),
            options: vec![],
            writable: true,
            menu: Menu::Configuration,
            group: "Charger".into(),
        });
        let id = d.id;
        let (bus, state) = StubBus::new(0.0);
        state
            .values
            .lock()
            .unwrap()
            .insert((id, 0x015), Value::Boolean(true));
        state.writable.lock().unwrap().insert((id, 0x015), true);
        state
            .values
            .lock()
            .unwrap()
            .insert((id, 0x005), Value::Float(18.0));
        state.writable.lock().unwrap().insert((id, 0x005), true);
        let mut m = Mapping::new();
        let mut dm = DeviceMapping::default();
        dm.fields.insert(
            field_key(0x015),
            FieldMapping {
                path: "electrical.chargers.x.enabled".into(),
                invert: true,
                put: true,
                ..Default::default()
            },
        );
        dm.fields.insert(
            field_key(0x005),
            FieldMapping {
                path: "electrical.batteries.x.temperature".into(),
                ..Default::default()
            },
        );
        m.devices.insert("MLI-1".into(), dm);
        let (shared, _) = shared_with(bus, vec![d], m);

        let r = send(
            &shared,
            "PUT",
            "/api/devices/MLI-1/fields/0x015",
            json!({"value": true}),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["applied"], false, "enabled=true is Standby=false");
        assert_eq!(r.body["published"], true);
        assert_eq!(
            shared.values.lock().unwrap()[&(id, 0x015)],
            Value::Boolean(false)
        );
        assert_eq!(state.writes.lock().unwrap()[0].2, Value::Boolean(false));

        // A number arrives in SI and lands in the device unit.
        let r = send(
            &shared,
            "PUT",
            "/api/devices/MLI-1/fields/0x005",
            json!({"value": 293.15}),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert!((r.body["applied"].as_f64().unwrap() - 20.0).abs() < 1e-3);
        assert!((r.body["published"].as_f64().unwrap() - 293.15).abs() < 1e-3);

        // Nonsense is a client error with the decoder's explanation.
        let r = send(
            &shared,
            "PUT",
            "/api/devices/MLI-1/fields/0x005",
            json!({"value": "hot"}),
        );
        assert_eq!(r.status, 400);
        assert!(
            r.body["error"]
                .as_str()
                .unwrap()
                .contains("expected a number")
        );
        assert_eq!(
            handle(
                &shared,
                &Request::new("PUT", "/api/devices/MLI-1/fields/0x005", b"nope")
            )
            .status,
            400
        );
        assert_eq!(
            send(
                &shared,
                "PUT",
                "/api/devices/MLI-1/fields/0x0FF",
                json!({"value": 1})
            )
            .status,
            404
        );
    }

    /// A read-only field is 403 with a hint; with a login in the body the
    /// write is retried after logging in, and a wrong code is also 403.
    #[test]
    fn a_read_only_field_needs_a_login() {
        let d = mli();
        let id = d.id;
        let (bus, state) = StubBus::new(1234.0);
        state
            .values
            .lock()
            .unwrap()
            .insert((id, 0x001), Value::Float(24.0));
        let (shared, _) = shared_with(bus, vec![d], Mapping::new());
        let url = "/api/devices/MLI-1/fields/0x001";
        let r = send(&shared, "PUT", url, json!({"value": 25.0}));
        assert_eq!(r.status, 403);
        assert_eq!(r.body["needs"], "login");
        let r = send(
            &shared,
            "PUT",
            url,
            json!({"value": 25.0, "login": {"level": "installer", "code": 1.0}}),
        );
        assert_eq!(r.status, 403);
        assert!(r.body["error"].as_str().unwrap().contains("rejected"));
        let r = send(
            &shared,
            "PUT",
            url,
            json!({"value": 25.0, "login": {"level": "banana", "code": 1234.0}}),
        );
        assert_eq!(r.status, 400);
        let r = send(
            &shared,
            "PUT",
            url,
            json!({"value": 25.0, "login": {"level": "Installer", "code": 1234.0}}),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body["applied"], 25.0);
        // Unmapped: no conversion, so what is applied is what was sent.
        assert_eq!(r.body["published"], 25.0);
    }

    #[test]
    fn raw_json_renders_every_value_kind() {
        assert_eq!(raw_json(&Value::Float(1.5)), json!(1.5));
        assert_eq!(raw_json(&Value::Float(f32::NAN)), Json::Null);
        assert_eq!(raw_json(&Value::Boolean(true)), json!(true));
        assert_eq!(
            raw_json(&Value::List {
                index: 1,
                options: vec!["Off".into(), "On".into()]
            }),
            json!("On")
        );
        assert_eq!(
            raw_json(&Value::List {
                index: 1,
                options: vec![]
            }),
            json!(1)
        );
        assert_eq!(
            raw_json(&Value::Text {
                sid: 1,
                text: "Nav".into()
            }),
            json!("Nav")
        );
        assert_eq!(
            raw_json(&Value::Time(masterbus::Time {
                sec: 30,
                min: 1,
                hour: 0,
                days: 0
            })),
            json!(90.0)
        );
        assert_eq!(
            raw_json(&Value::Time(masterbus::Time {
                sec: -1,
                min: 0,
                hour: 0,
                days: 0
            })),
            Json::Null
        );
        assert_eq!(
            raw_json(&Value::Date(masterbus::Date {
                day: 3,
                mon: 9,
                year: 2026
            })),
            json!("2026-09-03")
        );
        assert_eq!(raw_json(&Value::Invalid), Json::Null);
        assert_eq!(parse_level("MV Service"), Some(AccessLevel::MvService));
        assert_eq!(parse_level("end-user"), Some(AccessLevel::EndUser));
    }

    /// The real server: routing, the bearer header and the JSON content type,
    /// over a loopback socket.
    #[test]
    fn the_server_answers_over_http_with_a_bearer_check() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let (shared, _) = shared_with(StubBus::plain(), vec![mli()], Mapping::new());
        let mut s = Arc::try_unwrap(shared).ok().unwrap();
        s.token = Some("tok".into());
        let shared = Arc::new(s);
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let addr = server.server_addr().to_ip().unwrap();
        {
            let (server, shared) = (server.clone(), shared.clone());
            std::thread::spawn(move || serve(server, shared));
        }
        let call = |req: &str| -> String {
            let mut c = TcpStream::connect(addr).unwrap();
            c.write_all(req.as_bytes()).unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).unwrap();
            out
        };
        let r = call("GET /api/status HTTP/1.0\r\n\r\n");
        assert!(r.starts_with("HTTP/1.0 401"), "{r}");
        let r = call("GET /api/devices HTTP/1.0\r\nAuthorization: Bearer tok\r\n\r\n");
        assert!(r.starts_with("HTTP/1.0 200"), "{r}");
        assert!(r.contains("Content-Type: application/json"), "{r}");
        assert!(r.contains("\"serial\":\"MLI-1\""), "{r}");
        let body = r#"{"serial":"MLI-1","field":"0x001"}"#;
        let r = call(&format!(
            "POST /api/mapping/suggest HTTP/1.0\r\nAuthorization: Bearer tok\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ));
        assert!(r.contains("electrical.batteries.24vService.voltage"), "{r}");
        server.unblock();
    }
}
