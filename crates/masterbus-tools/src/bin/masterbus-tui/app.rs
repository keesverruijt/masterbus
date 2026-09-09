//! TUI application state and the logic that mutates it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::path::PathBuf;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use masterbus::{
    AccessLevel, DeviceIdentity, DeviceStatus, FieldId, FieldInfo, GroupInfo, MasterBus, Menu,
    Subscription, Value, VisualizationType, field_id,
};
use masterbus_tools::mapping::{DeviceMapping, FieldMapping, Mapping, field_key, parse_field_key};
use masterbus_tools::{seed, signalk, units};

/// Live-poll rate for the selected device's monitoring fields.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// The tabs the UI exposes inside a device, in display order. Position 0 is
/// always the Summary tab (device identity); the rest are the data tabs.
///
/// **Btm1 channel** (legacy MasterBus) exposes its groups via
/// Monitoring / Configuration / Service; some devices (e.g. the Magic-class
/// Nav Chg) report empty group lists on Btm1, so those tabs may be sparse
/// or empty depending on the device.
///
/// **Btm3 channel** (newer MasterBus revision) has no Btm1-style group
/// structure on the wire — it lives in a flat per-channel field-index
/// space, surfaced as the **Settings** tab.
///
/// Alarms / History tabs are intentionally absent here; the Menu variants
/// in core still exist so they can be restored once the Btm3 sub-channel
/// structure (class `0x1A` / `0x1B` on real address) is properly understood.
pub const TABS: [TabKind; 5] = [
    TabKind::Summary,
    TabKind::Menu(Menu::Monitoring),
    TabKind::Menu(Menu::Configuration),
    TabKind::Menu(Menu::Service),
    TabKind::Settings,
];

/// Which inside-a-device tab is showing. [`TabKind::Summary`] is used only
/// as the right-pane preview / first-landing tab; [`TabKind::Settings`] is
/// the Btm3-flat tab built from `Device::all_fields()` filtered to fields
/// with the Btm3 channel bit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    /// Device identity + access level.
    Summary,
    /// One of the Btm1 wire menus. Btm1 groups are enumerated via
    /// `Device::tab(menu)`.
    Menu(Menu),
    /// Flat list of every Btm3 field on the device. Built from
    /// `Device::all_fields()` filtered by `field_id::channel == Btm3`.
    Settings,
}

/// Device id → name, shared with the background name-backfill thread.
pub type Names = Arc<Mutex<HashMap<u32, String>>>;

/// Device id → full identity, filled by the same backfill thread. The mapping
/// editor keys on serial number, and matches "apply to this article" on the
/// article number, so it needs more than the name.
pub type Idents = Arc<Mutex<HashMap<u32, DeviceIdentity>>>;

/// A line in the field pane: either a group header or a field.
pub enum Row {
    Group(String),
    Field(FieldInfo),
}

/// Snapshot of a list/eventable field used by the `?` "show all values"
/// modal. Built from the selected field at open time so the modal renders
/// without having to look anything up live.
pub struct ValuesView {
    pub field_name: String,
    pub options: Vec<String>,
    /// The index currently set on the device (if known), so it can be
    /// highlighted in the option list.
    pub current: Option<i32>,
}

/// Which pane has the keyboard focus.
#[derive(PartialEq, Eq)]
pub enum Focus {
    Devices,
    Fields,
}

/// An in-progress edit of the selected field.
pub struct Editor {
    pub field: FieldId,
    pub name: String,
    pub kind: EditKind,
}

pub enum EditKind {
    /// Free-text numeric entry.
    Number(String),
    /// Pick one of a fixed set of options.
    Choice { options: Vec<String>, sel: usize },
    /// Free-text string entry; written via the string-table chunk protocol
    /// (`MasterBus::Device::write_string`) at `str_id`. Until we figure out
    /// how Text-field metadata exposes the writable sid, the caller picks
    /// it (see `begin_edit` for the hardcoded family-specific mapping).
    Text { str_id: u16, buf: String },
}

/// The fixed four access levels, in display order. Used by the login modal.
pub const LOGIN_LEVELS: [AccessLevel; 4] = [
    AccessLevel::EndUser,
    AccessLevel::Installer,
    AccessLevel::Distributor,
    AccessLevel::MvService,
];

/// What the login modal is currently doing.
pub enum LoginStage {
    /// Picking the target level from [`LOGIN_LEVELS`].
    PickLevel,
    /// Picking End User → confirmed; submit logout next.
    /// Or: picked a non-EndUser level and now collecting a password
    /// string. The library/UI does not validate the value; whatever the
    /// user types is converted to `f32` and sent on the wire as-is.
    EnterPassword { level: AccessLevel, buf: String },
}

/// A modal that picks an access level for the currently-selected device.
pub struct LoginPrompt {
    /// Target device id.
    pub device: u32,
    /// Highlighted option in [`LOGIN_LEVELS`].
    pub sel: usize,
    /// Level the device reports when the prompt opened (for display).
    pub current: Option<AccessLevel>,
    /// Two-step UX: pick a level, then (for non-EndUser) enter a password.
    pub stage: LoginStage,
}

/// A single-tab discovery running on a worker thread. Either a menu (groups)
/// or the flat field probe; the rendered result in both cases is a list of
/// rows, but the worker fetches them via different code paths.
pub struct Pending {
    pub id: u32,
    pub tab: TabKind,
    pub name: String,
    pub started: Instant,
    rx: Receiver<Vec<GroupInfo>>,
}

pub struct App {
    pub bus: MasterBus,
    pub device_ids: Vec<u32>,
    pub names: Names,
    pub dev_sel: usize,
    pub focus: Focus,
    pub cur_device: Option<u32>,
    /// Cached identity for the Summary tab.
    pub cur_info: Option<DeviceIdentity>,
    /// Cached access level for the open device (shown in the title; refreshed
    /// on device open, after login, and on Summary-tab visit).
    pub cur_access_level: Option<AccessLevel>,
    /// The currently-displayed tab.
    pub cur_tab: TabKind,
    /// Menus already discovered for `cur_device`.
    pub loaded_menus: HashSet<Menu>,
    /// Whether the flat field probe (`all_fields`) is loaded for `cur_device`.
    pub settings_loaded: bool,
    pub rows: Vec<Row>,
    pub row_sel: usize,
    pub values: HashMap<FieldId, Value>,
    pub sub: Option<Subscription>,
    pub editor: Option<Editor>,
    /// Read-only modal that lists every option of a list/eventable field. `?`
    /// opens it on the selected row; Esc/q closes. (Named `..._modal` to avoid
    /// colliding with the `values:` field-value cache below.)
    pub values_modal: Option<ValuesView>,
    pub login: Option<LoginPrompt>,
    pub pending: Option<Pending>,
    pub tick: usize,
    pub status: String,
    pub should_quit: bool,
    /// Whether the bottom log pane (fed by `tui-logger`) is visible. Toggled
    /// with `~`. Only relevant when logging lands in the TUI (no
    /// `MASTERBUS_TUI_LOG` redirect).
    pub show_logs: bool,
    /// Whether `tui-logger` is the active backend (and thus the pane is
    /// useful at all). Drives the status hint and the `~` key handler.
    pub logs_in_tui: bool,
    /// Identity of every device seen, for the mapping editor's serial and
    /// article lookups.
    pub idents: Idents,
    /// The mapping-editor session; `None` unless started with `--mapping`.
    pub mapping: Option<MappingSession>,
    /// Open path-editor modal.
    pub path_editor: Option<PathEditor>,
    /// Serial number of the open device, cached when it is opened.
    pub cur_serial: Option<String>,
    /// Signal K instance proposed for the open device.
    pub cur_instance: String,
}

impl App {
    /// Construct without blocking: seed from whatever devices have been heard so
    /// far; the rest arrive via `note_alive`, and names via the backfill thread.
    pub fn new(
        bus: MasterBus,
        names: Names,
        idents: Idents,
        mapping: Option<MappingSession>,
        logs_in_tui: bool,
    ) -> App {
        let device_ids: Vec<u32> = bus.devices().iter().map(|d| d.id()).collect();
        App {
            bus,
            device_ids,
            names,
            dev_sel: 0,
            focus: Focus::Devices,
            cur_device: None,
            cur_info: None,
            cur_access_level: None,
            cur_tab: TabKind::Summary,
            loaded_menus: HashSet::new(),
            settings_loaded: false,
            rows: Vec::new(),
            row_sel: 0,
            values: HashMap::new(),
            sub: None,
            editor: None,
            values_modal: None,
            login: None,
            pending: None,
            tick: 0,
            status: if logs_in_tui {
                "scanning bus… ↑/↓ select · Enter open · l login · ~ logs · q quit".into()
            } else {
                "scanning bus… ↑/↓ select · Enter open · l login · q quit".into()
            },
            should_quit: false,
            show_logs: false,
            logs_in_tui,
            idents,
            mapping,
            path_editor: None,
            cur_serial: None,
            cur_instance: String::new(),
        }
    }

    /// Toggle the bottom log pane (no-op when logs aren't routed to the TUI).
    pub fn toggle_logs(&mut self) {
        if self.logs_in_tui {
            self.show_logs = !self.show_logs;
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    // ---- device pane ------------------------------------------------------

    pub fn device_status(&self, id: u32) -> DeviceStatus {
        self.bus.device(id).status()
    }

    pub fn device_label(&self, id: u32) -> String {
        self.names
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    pub fn note_alive(&mut self, id: u32) {
        if !self.device_ids.contains(&id) {
            self.device_ids.push(id);
        }
    }

    pub fn move_device(&mut self, delta: i32) {
        if self.device_ids.is_empty() {
            return;
        }
        let n = self.device_ids.len() as i32;
        self.dev_sel = (self.dev_sel as i32 + delta).clamp(0, n - 1) as usize;
    }

    /// Drill into the selected device, landing on the Summary tab. The user
    /// can Tab into Monitoring / All Fields.
    pub fn open_device(&mut self) {
        let Some(&id) = self.device_ids.get(self.dev_sel) else {
            return;
        };
        self.cur_device = Some(id);
        self.loaded_menus.clear();
        self.settings_loaded = false;
        self.sub = None;
        self.rows.clear();
        self.values.clear();
        self.row_sel = 0;
        self.focus = Focus::Fields;
        self.cur_tab = TabKind::Summary;
        self.enter_summary();
    }

    /// Cycle to the next / previous tab.
    pub fn next_tab(&mut self) {
        self.cycle_tab(1);
    }
    pub fn prev_tab(&mut self) {
        self.cycle_tab(-1);
    }

    fn cycle_tab(&mut self, delta: i32) {
        if self.cur_device.is_none() || self.pending.is_some() {
            return;
        }
        let n = TABS.len() as i32;
        let cur = TABS.iter().position(|&t| t == self.cur_tab).unwrap_or(0) as i32;
        let next = ((cur + delta) % n + n) % n;
        self.switch_tab(TABS[next as usize]);
    }

    /// Switch to the Summary tab: device identity + access level, no field list.
    fn enter_summary(&mut self) {
        let Some(id) = self.cur_device else { return };
        self.cur_tab = TabKind::Summary;
        self.sub = None;
        self.rows.clear();
        self.row_sel = 0;
        self.cur_info = self.bus.device(id).identity().ok();
        // The mapping editor keys on serial, so cache it (and the proposed
        // Signal K instance) whenever identity is refreshed.
        if let Some(i) = &self.cur_info {
            self.cur_serial = Some(i.serial.clone()).filter(|x| !x.is_empty());
            self.cur_instance = seed::instance_of(&i.name, id);
            self.idents.lock().unwrap().insert(id, i.clone());
        }
        self.cur_access_level = self.bus.device(id).access_level().ok();
        self.status = format!(
            "{} / Summary — Tab switch · Esc back",
            self.device_label(id)
        );
    }

    fn switch_tab(&mut self, tab: TabKind) {
        let Some(id) = self.cur_device else { return };
        self.cur_tab = tab;
        match tab {
            TabKind::Summary => self.enter_summary(),
            TabKind::Menu(menu) => {
                if self.loaded_menus.contains(&menu) {
                    let groups = self.bus.device(id).tab_info(menu).unwrap_or_default();
                    self.show_groups(id, menu, groups);
                } else {
                    self.rows.clear();
                    self.row_sel = 0;
                    self.start_menu_discovery(id, menu);
                }
            }
            TabKind::Settings => {
                if self.settings_loaded {
                    let fields = self.bus.device(id).all_fields().unwrap_or_default();
                    self.show_settings(id, fields);
                } else {
                    self.rows.clear();
                    self.row_sel = 0;
                    self.start_settings_discovery(id);
                }
            }
        }
    }

    /// Spawn a worker to discover one menu's groups (UI shows a spinner).
    fn start_menu_discovery(&mut self, id: u32, menu: Menu) {
        let name = self.device_label(id);
        self.status = format!("discovering {} / {}…", name, menu_label(menu));
        let (tx, rx) = bounded(1);
        let bus = self.bus.clone();
        std::thread::spawn(move || {
            if let Ok(groups) = bus.device(id).tab_info(menu) {
                let _ = tx.send(groups);
            }
        });
        self.pending = Some(Pending {
            id,
            tab: TabKind::Menu(menu),
            name,
            started: Instant::now(),
            rx,
        });
    }

    /// Spawn a worker to probe the device's full field-index space and
    /// keep only the Btm3 fields for the Settings tab.
    fn start_settings_discovery(&mut self, id: u32) {
        let name = self.device_label(id);
        self.status = format!("probing Btm3 fields of {}…", name);
        let (tx, rx) = bounded(1);
        let bus = self.bus.clone();
        std::thread::spawn(move || {
            if let Ok(all) = bus.device(id).all_fields() {
                // Filter to Btm3 only; Btm1 fields show up under the
                // Monitoring / Configuration / Service tabs (with their
                // proper Btm1 group structure where available).
                let fields: Vec<FieldInfo> = all
                    .into_iter()
                    .filter(|f| field_id::channel(f.index) == masterbus::Channel::Btm3)
                    .collect();
                // Reuse the GroupInfo carrier with a single synthetic group so
                // the existing `poll_pending` / `show_*` plumbing stays
                // uniform.
                let one = GroupInfo {
                    id: -1,
                    name: String::new(),
                    menu: Menu::Other(0x01),
                    fields,
                };
                let _ = tx.send(vec![one]);
            }
        });
        self.pending = Some(Pending {
            id,
            tab: TabKind::Settings,
            name,
            started: Instant::now(),
            rx,
        });
    }

    /// Whether a tab discovery is in flight.
    pub fn discovering(&self) -> bool {
        self.pending.is_some()
    }

    /// (name, tab, elapsed seconds) of the in-flight discovery, if any.
    pub fn pending_info(&self) -> Option<(&str, TabKind, u64)> {
        self.pending
            .as_ref()
            .map(|p| (p.name.as_str(), p.tab, p.started.elapsed().as_secs()))
    }

    /// Check the discovery worker; when the result arrives, show it.
    pub fn poll_pending(&mut self) {
        let Some(p) = &self.pending else { return };
        match p.rx.try_recv() {
            Ok(groups) => {
                let (id, tab) = (p.id, p.tab);
                self.pending = None;
                if let Ok(n) = self.bus.device(id).name() {
                    self.names.lock().unwrap().insert(id, n);
                }
                match tab {
                    TabKind::Menu(menu) => {
                        self.loaded_menus.insert(menu);
                        self.show_groups(id, menu, groups);
                    }
                    TabKind::Settings => {
                        self.settings_loaded = true;
                        // The worker packs the flat result into a single
                        // synthetic group; pull the fields back out.
                        let fields = groups
                            .into_iter()
                            .next()
                            .map(|g| g.fields)
                            .unwrap_or_default();
                        self.show_settings(id, fields);
                    }
                    TabKind::Summary => {} // never spawned
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.focus = Focus::Devices;
                self.status = "discovery failed".into();
            }
        }
    }

    /// Abandon the in-flight discovery and return to the device list.
    pub fn cancel_pending(&mut self) {
        self.pending = None;
        self.cur_device = None;
        self.focus = Focus::Devices;
        self.status = "discovery cancelled".into();
    }

    fn show_groups(&mut self, id: u32, menu: Menu, groups: Vec<GroupInfo>) {
        self.build_rows(&groups);
        self.values.clear();
        // Subscribe on every tab — Configuration / Service have editable
        // Text/DropDown fields whose current values (including Btm3 sids for
        // Text VIZ) must be cached before the user can edit them.
        let fields: Vec<FieldId> = groups
            .iter()
            .flat_map(|g| g.fields.iter().map(|f| f.index))
            .collect();
        self.sub = if !fields.is_empty() {
            Some(self.bus.subscribe(id, fields, POLL_INTERVAL, false))
        } else {
            None
        };
        self.row_sel = 0;
        self.select_first_field();
        self.status = format!(
            "{} / {} — Tab switch · Enter edit · ? values{} · Esc back",
            self.device_label(id),
            menu_label(menu),
            if self.mapping_mode() {
                " · + map · - unmap · a apply to article · w write"
            } else {
                ""
            }
        );
    }

    fn show_settings(&mut self, id: u32, fields: Vec<FieldInfo>) {
        self.rows.clear();
        for f in &fields {
            self.rows.push(Row::Field(f.clone()));
        }
        self.values.clear();
        // Subscribe to every Btm3 field so values stream in (mostly via
        // passive `0x0B` pushes on the wire). Use the standard live-poll
        // interval; the engine will rate-limit redundant work.
        let ids: Vec<FieldId> = fields.iter().map(|f| f.index).collect();
        self.sub = if ids.is_empty() {
            None
        } else {
            Some(self.bus.subscribe(id, ids, POLL_INTERVAL, false))
        };
        self.row_sel = 0;
        self.select_first_field();
        self.status = format!(
            "{} / Settings ({} Btm3 fields) — Tab switch · Enter edit · ? values · Esc back",
            self.device_label(id),
            fields.len()
        );
    }

    fn build_rows(&mut self, groups: &[GroupInfo]) {
        self.rows.clear();
        for g in groups {
            self.rows.push(Row::Group(g.name.clone()));
            for f in &g.fields {
                self.rows.push(Row::Field(f.clone()));
            }
        }
    }

    pub fn back_to_devices(&mut self) {
        self.focus = Focus::Devices;
        self.sub = None;
        self.pending = None;
        self.rows.clear();
        self.loaded_menus.clear();
        self.settings_loaded = false;
        self.cur_tab = TabKind::Summary;
        self.cur_info = None;
        self.cur_access_level = None;
        self.cur_device = None;
        self.status = "↑/↓ select · Enter open · q quit".into();
    }

    // ---- field pane -------------------------------------------------------

    fn select_first_field(&mut self) {
        if let Some(i) = self.rows.iter().position(|r| matches!(r, Row::Field(_))) {
            self.row_sel = i;
            self.refresh_selected();
        }
    }

    pub fn move_row(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        let mut i = self.row_sel as i32;
        loop {
            i += delta;
            if i < 0 || i >= n {
                return; // keep current selection at the edge
            }
            if matches!(self.rows[i as usize], Row::Field(_)) {
                self.row_sel = i as usize;
                break;
            }
        }
        self.refresh_selected();
    }

    fn selected_field(&self) -> Option<&FieldInfo> {
        match self.rows.get(self.row_sel) {
            Some(Row::Field(f)) => Some(f),
            _ => None,
        }
    }

    /// Read the selected field once if we don't have a cached value yet.
    fn refresh_selected(&mut self) {
        let Some(idx) = self.selected_field().map(|f| f.index) else {
            return;
        };
        self.ensure_value(idx);
    }

    fn ensure_value(&mut self, index: FieldId) {
        if self.values.contains_key(&index) {
            return;
        }
        let Some(id) = self.cur_device else { return };
        if let Ok(v) = self.bus.device(id).field(index).value() {
            self.values.insert(index, v);
        }
    }

    /// Force a fresh read of the selected field.
    pub fn reread_selected(&mut self) {
        if let Some(idx) = self.selected_field().map(|f| f.index) {
            self.values.remove(&idx);
            self.ensure_value(idx);
            self.status = "re-read".into();
        }
    }

    pub fn pump_subscription(&mut self) {
        let Some(sub) = &self.sub else { return };
        let mut updates = Vec::new();
        while let Some(u) = sub.try_recv() {
            updates.push((u.field, u.value));
        }
        for (f, v) in updates {
            self.values.insert(f, v);
        }
    }

    // ---- editing ----------------------------------------------------------

    pub fn begin_edit(&mut self) {
        let Some(info) = self.selected_field().cloned() else {
            return;
        };
        if !info.writeable {
            self.status = format!("{} is read-only", info.name);
            return;
        }
        use VisualizationType as V;
        match info.viz_type {
            V::CheckBox | V::ToggleButton | V::PushButton => {
                let cur = matches!(self.values.get(&info.index), Some(Value::Boolean(true)));
                self.write(info.index, Value::Boolean(!cur));
            }
            V::Float => {
                let cur = match self.values.get(&info.index) {
                    Some(Value::Float(f)) if !f.is_nan() => format!("{f}"),
                    _ => String::new(),
                };
                self.editor = Some(Editor {
                    field: info.index,
                    name: info.name.clone(),
                    kind: EditKind::Number(cur),
                });
            }
            V::Radio | V::DropDown => {
                let options = info.options.clone();
                let sel = match self.values.get(&info.index) {
                    Some(Value::List { index, .. }) => *index as usize,
                    _ => 0_usize,
                };
                let sel = sel.min(options.len().saturating_sub(1));
                self.editor = Some(Editor {
                    field: info.index,
                    name: info.name.clone(),
                    kind: EditKind::Choice { options, sel },
                });
            }
            V::Text => {
                // The cached value carries (sid, current text); pre-fill the
                // buffer so the user edits the existing name rather than
                // typing from scratch.
                let (str_id, buf) = match self.values.get(&info.index) {
                    Some(Value::Text { sid, text }) => (*sid, text.clone()),
                    _ => {
                        self.status = format!("{}: value not loaded yet", info.name);
                        return;
                    }
                };
                self.editor = Some(Editor {
                    field: info.index,
                    name: info.name.clone(),
                    kind: EditKind::Text { str_id, buf },
                });
            }
            _ => self.status = format!("{}: not editable in this demo", info.name),
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editor = None;
        self.status = "edit cancelled".into();
    }

    /// Read-only "show all values" modal for the selected list/eventable
    /// field. No-op on non-list fields. The current index (if cached) is
    /// passed in so the modal can highlight it.
    pub fn open_values(&mut self) {
        let Some(info) = self.selected_field().cloned() else {
            return;
        };
        if info.options.is_empty() {
            self.status = format!("{}: no list values to show", info.name);
            return;
        }
        let current = match self.values.get(&info.index) {
            Some(Value::List { index, .. }) | Some(Value::Eventable { index, .. }) => Some(*index),
            _ => None,
        };
        self.values_modal = Some(ValuesView {
            field_name: info.name,
            options: info.options,
            current,
        });
    }

    pub fn close_values(&mut self) {
        self.values_modal = None;
    }

    pub fn values_open(&self) -> bool {
        self.values_modal.is_some()
    }

    pub fn commit_edit(&mut self) {
        let Some(ed) = self.editor.take() else { return };
        match ed.kind {
            EditKind::Number(buf) => match buf.trim().parse::<f32>() {
                Ok(f) => self.write(ed.field, Value::Float(f)),
                Err(_) => self.status = format!("'{buf}' is not a number"),
            },
            EditKind::Choice { options, sel } => {
                self.write(
                    ed.field,
                    Value::List {
                        index: sel as i32,
                        options,
                    },
                );
            }
            EditKind::Text { str_id, buf } => {
                self.write(
                    ed.field,
                    Value::Text {
                        sid: str_id,
                        text: buf,
                    },
                );
            }
        }
    }

    pub fn editor_char(&mut self, c: char) {
        match &mut self.editor {
            Some(Editor {
                kind: EditKind::Number(buf),
                ..
            }) if c.is_ascii_digit() || c == '.' || c == '-' => {
                buf.push(c);
            }
            // Text fields are constrained on the wire: printable ASCII only,
            // ≤16 bytes. Enforce here so the user can't even type past it.
            Some(Editor {
                kind: EditKind::Text { buf, .. },
                ..
            }) if (c.is_ascii_graphic() || c == ' ')
                && buf.len() < masterbus::MAX_EDITABLE_TEXT_BYTES =>
            {
                buf.push(c);
            }
            _ => {}
        }
    }

    pub fn editor_backspace(&mut self) {
        match &mut self.editor {
            Some(Editor {
                kind: EditKind::Number(buf),
                ..
            }) => {
                buf.pop();
            }
            Some(Editor {
                kind: EditKind::Text { buf, .. },
                ..
            }) => {
                buf.pop();
            }
            _ => {}
        }
    }

    pub fn editor_choice_move(&mut self, delta: i32) {
        if let Some(Editor {
            kind: EditKind::Choice { options, sel },
            ..
        }) = &mut self.editor
        {
            if options.is_empty() {
                return;
            }
            let n = options.len() as i32;
            *sel = (((*sel as i32 + delta) % n + n) % n) as usize;
        }
    }

    fn write(&mut self, index: FieldId, value: Value) {
        let Some(id) = self.cur_device else { return };
        match self.bus.device(id).field(index).set(value) {
            Ok(v) => {
                self.values.insert(index, v);
                self.status = "set ok".into();
            }
            Err(e) => self.status = format!("set failed: {e}"),
        }
    }

    pub fn editing(&self) -> bool {
        self.editor.is_some()
    }

    // ---- login modal ------------------------------------------------------

    /// True when the login picker is active and owns the keys.
    pub fn login_modal(&self) -> bool {
        self.login.is_some()
    }

    /// True when the modal is collecting the password (rather than picking a
    /// level) — the key handler routes printable chars / Backspace here.
    pub fn login_at_password_stage(&self) -> bool {
        matches!(
            &self.login,
            Some(LoginPrompt {
                stage: LoginStage::EnterPassword { .. },
                ..
            })
        )
    }

    /// Open the login modal for the currently-targeted device. When focused on
    /// the device list this targets the highlighted device; when inside a
    /// device's tabs it targets that device.
    pub fn open_login(&mut self) {
        let Some(device) = self
            .cur_device
            .or_else(|| self.device_ids.get(self.dev_sel).copied())
        else {
            return;
        };
        let current = self.bus.device(device).access_level().ok();
        let sel = current
            .and_then(|l| LOGIN_LEVELS.iter().position(|&x| x == l))
            .unwrap_or(0);
        self.login = Some(LoginPrompt {
            device,
            sel,
            current,
            stage: LoginStage::PickLevel,
        });
        self.status = "select access level — ↑/↓ pick · Enter next · Esc cancel".into();
    }

    pub fn login_move(&mut self, delta: i32) {
        if let Some(LoginPrompt {
            stage: LoginStage::PickLevel,
            sel,
            ..
        }) = &mut self.login
        {
            let n = LOGIN_LEVELS.len() as i32;
            *sel = (((*sel as i32 + delta) % n + n) % n) as usize;
        }
    }

    pub fn cancel_login(&mut self) {
        self.login = None;
        self.status = "login cancelled".into();
    }

    /// Append a printable char to the password buffer.
    pub fn login_char(&mut self, c: char) {
        if let Some(LoginPrompt {
            stage: LoginStage::EnterPassword { buf, .. },
            ..
        }) = &mut self.login
            && c.is_ascii_graphic()
        {
            buf.push(c);
        }
    }

    /// Pop the last char of the password buffer.
    pub fn login_backspace(&mut self) {
        if let Some(LoginPrompt {
            stage: LoginStage::EnterPassword { buf, .. },
            ..
        }) = &mut self.login
        {
            buf.pop();
        }
    }

    /// Enter on the login modal. In `PickLevel`: End User submits a logout
    /// straight away; any other level advances to the password-entry stage.
    /// In `EnterPassword`: parse the buffer as `f32` silently and attempt the
    /// login. If the device reports the same level it was at before, the
    /// password was rejected.
    pub fn commit_login(&mut self) {
        let Some(mut p) = self.login.take() else {
            return;
        };
        match &p.stage {
            LoginStage::PickLevel => {
                let level = LOGIN_LEVELS[p.sel];
                if level == AccessLevel::EndUser {
                    let prev = p.current;
                    self.apply_login_result(
                        p.device,
                        level,
                        self.bus.device(p.device).logout(),
                        prev,
                    );
                } else {
                    // Stay in the modal; collect a password.
                    p.stage = LoginStage::EnterPassword {
                        level,
                        buf: String::new(),
                    };
                    self.status = "enter password — type · Enter submit · Esc cancel".into();
                    self.login = Some(p);
                }
            }
            LoginStage::EnterPassword { level, buf } => {
                let level = *level;
                let prev = p.current;
                // Silent parse — any non-numeric input becomes 0.0.
                let code = buf.parse::<f32>().unwrap_or(0.0);
                let r = self.bus.device(p.device).login(level, code);
                self.apply_login_result(p.device, level, r, prev);
            }
        }
    }

    /// Common post-login wiring: status line, optional schema refresh, the
    /// "that seems to be an incorrect password" branch.
    fn apply_login_result(
        &mut self,
        device: u32,
        attempted: AccessLevel,
        reported: Result<AccessLevel, masterbus::Error>,
        prev: Option<AccessLevel>,
    ) {
        match reported {
            Ok(reported) => {
                self.status = if Some(reported) == prev && attempted != AccessLevel::EndUser {
                    "that seems to be an incorrect password".to_string()
                } else {
                    format!("device 0x{:06X} → {}", device, level_label(reported))
                };
                if self.cur_device == Some(device) {
                    self.cur_access_level = Some(reported);
                    self.loaded_menus.clear();
                    self.settings_loaded = false;
                    self.values.clear();
                    self.sub = None;
                    match self.cur_tab {
                        TabKind::Summary => {
                            self.cur_info = self.bus.device(device).identity().ok();
                        }
                        tab => {
                            self.rows.clear();
                            self.row_sel = 0;
                            self.switch_tab(tab);
                        }
                    }
                }
            }
            Err(e) => {
                self.status = format!("login on 0x{device:06X} failed: {e}");
            }
        }
    }
}

pub fn menu_label(menu: Menu) -> String {
    match menu {
        Menu::Monitoring => "Monitoring".into(),
        Menu::Configuration => "Configuration".into(),
        Menu::Service => "Service".into(),
        Menu::Alarm => "Alarms".into(),
        Menu::History => "History".into(),
        Menu::Other(s) => format!("Menu {s:#04x}"),
    }
}

/// User-facing label for a [`TabKind`] (shown in the tab bar and status line).
pub fn tab_label(tab: TabKind) -> String {
    match tab {
        TabKind::Summary => "Summary".into(),
        TabKind::Menu(m) => menu_label(m),
        TabKind::Settings => "Settings".into(),
    }
}

/// User-facing label for an access level (matches MasterAdjust's terminology).
pub fn level_label(level: AccessLevel) -> &'static str {
    match level {
        AccessLevel::EndUser => "End User",
        AccessLevel::Installer => "Installer",
        AccessLevel::Distributor => "Distributor",
        AccessLevel::MvService => "MV Service",
    }
}

// ---- mapping editor ------------------------------------------------------

/// The mapping-editor session, present only when the TUI was started with
/// `--mapping`.
///
/// The whole file is held in memory and written back on demand. That is what
/// preserves entries for devices which are switched off or off the bus today:
/// nothing is rebuilt from the live bus, only the entries the user touches are
/// changed. Issue #3 called that out as a hazard of the old rewrite-everything
/// mapping file.
pub struct MappingSession {
    /// Where the file lives; also where `w` writes it back.
    pub path: PathBuf,
    /// The whole file, including devices that are not on this bus.
    pub map: Mapping,
    /// Unsaved changes.
    pub dirty: bool,
    /// Set once the user has been warned about quitting with unsaved changes.
    pub quit_armed: bool,
}

/// Where a pre-filled path suggestion came from, so the editor can say how much
/// to trust it. "Known for this model" and "guessed from a name" deserve
/// different amounts of scrutiny from whoever is about to press Enter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Already mapped; this is an edit.
    Existing,
    /// Proposed by the suggestion machinery, at the given confidence.
    Suggested(seed::Tier),
    /// Nothing to go on; the user is typing from scratch.
    Blank,
}

/// An in-progress edit of one field's Signal K path.
pub struct PathEditor {
    /// Which field is being mapped.
    pub field: FieldId,
    /// Its name, for the modal title.
    pub field_name: String,
    /// Its unit, to derive and display the conversion.
    pub unit: String,
    /// The path being typed.
    pub buf: String,
    /// Whether to publish the boolean negated.
    pub invert: bool,
    /// Where `buf` was seeded from.
    pub origin: Origin,
}

impl PathEditor {
    /// The conversion the current path implies, and a human description.
    /// `None` means the units cannot be reconciled — the entry would be
    /// skipped at runtime, so the editor says so before it is saved.
    pub fn conversion_hint(&self) -> Option<String> {
        let leaf = signalk::leaf_unit(&self.buf);
        let conv = units::conversion(&self.unit, leaf)?;
        Some(match (leaf, conv.is_identity()) {
            (None, _) => "no unit metadata for this leaf".into(),
            (Some(u), true) => format!("{u}, unchanged"),
            (Some(u), false) => format!(
                "→ {u} (×{} {}{})",
                conv.scale,
                if conv.offset >= 0.0 { "+" } else { "−" },
                conv.offset.abs()
            ),
        })
    }
}

impl App {
    /// Whether the mapping editor is active.
    pub fn mapping_mode(&self) -> bool {
        self.mapping.is_some()
    }

    /// Whether the path editor modal is open.
    pub fn path_editing(&self) -> bool {
        self.path_editor.is_some()
    }

    /// Serial number of a device, once identity discovery has reached it.
    pub fn serial_of(&self, id: u32) -> Option<String> {
        self.idents
            .lock()
            .unwrap()
            .get(&id)
            .map(|i| i.serial.clone())
            .filter(|s| !s.is_empty())
    }

    /// How many fields of a device the mapping publishes. `None` when there is
    /// no mapping session or the device's serial is not known yet.
    pub fn mapped_count(&self, id: u32) -> Option<usize> {
        let s = self.mapping.as_ref()?;
        let serial = self.serial_of(id)?;
        Some(s.map.devices.get(&serial).map_or(0, |d| d.fields.len()))
    }

    /// The Signal K path a field currently publishes to, if any.
    pub fn mapped_path(&self, field: FieldId) -> Option<&str> {
        let s = self.mapping.as_ref()?;
        let serial = self.cur_serial.as_ref()?;
        s.map.field(serial, field).map(|f| f.path.as_str())
    }

    /// Open the path editor on the selected field, pre-filled with the existing
    /// mapping, else with a heuristic suggestion, else empty.
    pub fn begin_map(&mut self) {
        if self.mapping.is_none() {
            return;
        }
        let Some(field) = self.selected_field().cloned() else {
            return;
        };
        let Some(serial) = self.cur_serial.clone() else {
            self.status = "this device has not reported a serial number yet".into();
            return;
        };
        let existing = self
            .mapping
            .as_ref()
            .and_then(|s| s.map.field(&serial, field.index))
            .cloned();
        let (buf, invert, origin) = match existing {
            Some(fm) => (fm.path, fm.invert, Origin::Existing),
            None => {
                let (name, article, firmware) = self
                    .cur_info
                    .as_ref()
                    .map(|i| (i.name.clone(), i.article.clone(), i.firmware.clone()))
                    .unwrap_or_default();
                let instance = self.cur_instance.clone();
                match seed::suggest_best(
                    &article,
                    &firmware,
                    seed::class_of(&name),
                    &instance,
                    field.index,
                    &field.name,
                    &field.unit,
                ) {
                    Some((s, tier)) => (s.path, s.invert, Origin::Suggested(tier)),
                    None => (String::new(), false, Origin::Blank),
                }
            }
        };
        self.path_editor = Some(PathEditor {
            field: field.index,
            field_name: field.name.clone(),
            unit: field.unit.clone(),
            buf,
            invert,
            origin,
        });
    }

    /// Remove the selected field's mapping.
    pub fn unmap_selected(&mut self) {
        let Some(field) = self.selected_field().map(|f| f.index) else {
            return;
        };
        let Some(serial) = self.cur_serial.clone() else {
            return;
        };
        let key = field_key(field);
        let Some(s) = self.mapping.as_mut() else {
            return;
        };
        if let Some(d) = s.map.devices.get_mut(&serial)
            && d.fields.remove(&key).is_some()
        {
            s.dirty = true;
            self.status = format!("unmapped {key}");
        }
    }

    /// Commit the path editor.
    pub fn commit_map(&mut self) {
        let Some(ed) = self.path_editor.take() else {
            return;
        };
        let path = ed.buf.trim().to_string();
        let Some(serial) = self.cur_serial.clone() else {
            return;
        };
        if path.is_empty() {
            self.status = "empty path; nothing changed".into();
            return;
        }
        // Refuse only what cannot work at runtime. An unknown leaf is allowed
        // (a custom path is a legitimate choice) but a unit pair that cannot be
        // reconciled would be skipped by the sidecar, so it is rejected here
        // where the user can see why.
        if units::conversion(&ed.unit, signalk::leaf_unit(&path)).is_none() {
            self.status = format!(
                "{:?} cannot be converted to what {path} expects — not saved",
                ed.unit
            );
            self.path_editor = Some(ed);
            return;
        }
        let identity = self.cur_info.clone();
        let instance = self.cur_instance.clone();
        let Some(s) = self.mapping.as_mut() else {
            return;
        };
        let entry = s.map.devices.entry(serial).or_default();
        if let Some(i) = &identity {
            entry.article = i.article.clone();
            entry.firmware = i.firmware.clone();
            entry.name = i.name.clone();
        }
        // Record the instance the path actually uses, not the one proposed for
        // the device. People rename freely here — an `INT Nav Chg` gets mapped
        // onto `electrical.chargers.nav-battery` because that is what it
        // charges — and `instance` is what "apply to this article" substitutes.
        // Left stale, that copy would substitute nothing and hand two devices
        // the same Signal K node.
        match signalk::instance_of(&path) {
            Some(used) => entry.instance = used,
            None if entry.instance.is_empty() => entry.instance = instance,
            None => {}
        }
        entry.fields.insert(
            field_key(ed.field),
            FieldMapping {
                path: path.clone(),
                invert: ed.invert,
            },
        );
        s.dirty = true;
        self.status = format!("{} → {path}", field_key(ed.field));
    }

    pub fn cancel_map(&mut self) {
        self.path_editor = None;
    }

    pub fn map_editor_char(&mut self, c: char) {
        if let Some(ed) = self.path_editor.as_mut() {
            ed.buf.push(c);
        }
    }

    pub fn map_editor_backspace(&mut self) {
        if let Some(ed) = self.path_editor.as_mut() {
            ed.buf.pop();
        }
    }

    /// Toggle the invert flag from inside the editor.
    pub fn map_editor_toggle_invert(&mut self) {
        if let Some(ed) = self.path_editor.as_mut() {
            ed.invert = !ed.invert;
        }
    }

    /// Copy the open device's mapping onto every other device with the same
    /// article, substituting each target's own instance into the paths.
    ///
    /// Without this nobody finishes the job: the bus in #6 has ten batteries on
    /// two articles and four identical chargers. Fields the target device does
    /// not have are skipped, which is what keeps a cluster master's extra
    /// fields from being forced onto a plain member.
    pub fn apply_to_article(&mut self) {
        let Some(src_serial) = self.cur_serial.clone() else {
            return;
        };
        let Some(src) = self
            .mapping
            .as_ref()
            .and_then(|s| s.map.devices.get(&src_serial).cloned())
        else {
            self.status = "map at least one field on this device first".into();
            return;
        };
        if src.article.is_empty() {
            self.status = "this device reports no article number to match on".into();
            return;
        }

        // Collect the targets: same article, different serial, known fields.
        let mut targets: Vec<CopyTarget> = Vec::new();
        {
            let idents = self.idents.lock().unwrap();
            for id in &self.device_ids {
                let Some(ident) = idents.get(id) else {
                    continue;
                };
                if ident.article != src.article
                    || ident.serial == src_serial
                    || ident.serial.is_empty()
                {
                    continue;
                }
                let have: HashSet<FieldId> = self
                    .bus
                    .device(*id)
                    .tab_info(Menu::Monitoring)
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|g| g.fields.iter().map(|f| f.index))
                    .collect();
                targets.push(CopyTarget {
                    instance: seed::instance_of(&ident.name, *id),
                    have,
                    ident: ident.clone(),
                });
            }
        }
        if targets.is_empty() {
            self.status = format!("no other device with article {}", src.article);
            return;
        }

        let Some(s) = self.mapping.as_mut() else {
            return;
        };
        let (copied, skipped) = copy_to_targets(&mut s.map, &src, &targets);
        s.dirty = true;
        self.status = format!(
            "copied {copied} mapping(s) to devices with article {}{}",
            src.article,
            if skipped > 0 {
                format!("; {skipped} field(s) absent on some targets")
            } else {
                String::new()
            }
        );
    }

    /// Write the mapping file.
    pub fn save_mapping(&mut self) {
        let Some(s) = self.mapping.as_mut() else {
            return;
        };
        match s.map.save(&s.path) {
            Ok(()) => {
                s.dirty = false;
                self.status = format!("wrote {} ({} field(s))", s.path.display(), s.map.len());
            }
            Err(e) => self.status = format!("could not write {}: {e}", s.path.display()),
        }
    }

    /// Quit, refusing once if there are unsaved changes.
    pub fn quit_checked(&mut self) {
        let unsaved = self.mapping.as_ref().is_some_and(|s| s.dirty);
        let armed = self.mapping.as_ref().is_some_and(|s| s.quit_armed);
        if unsaved && !armed {
            if let Some(s) = self.mapping.as_mut() {
                s.quit_armed = true;
            }
            self.status = "unsaved mapping changes — w to write, q again to discard".into();
            return;
        }
        self.should_quit = true;
    }
}

/// A device the open device's mapping can be copied onto.
pub struct CopyTarget {
    /// Signal K instance proposed for it, used when it has no entry yet.
    pub instance: String,
    /// The monitoring field ids it actually has.
    pub have: HashSet<FieldId>,
    /// Its identity, recorded into the new entry.
    pub ident: DeviceIdentity,
}

/// Copy one device's field mappings onto every target, substituting each
/// target's own Signal K instance into the paths. Returns (copied, skipped).
///
/// Fields the target does not have are skipped rather than written blind. That
/// is what keeps a cluster master's extra fields off a plain member of the same
/// article, which is the case the bus in #6 actually contains.
fn copy_to_targets(
    map: &mut Mapping,
    src: &DeviceMapping,
    targets: &[CopyTarget],
) -> (usize, usize) {
    let mut copied = 0usize;
    let mut skipped = 0usize;
    for t in targets {
        let entry = map.devices.entry(t.ident.serial.clone()).or_default();
        entry.article = t.ident.article.clone();
        entry.firmware = t.ident.firmware.clone();
        entry.name = t.ident.name.clone();
        if entry.instance.is_empty() {
            entry.instance = t.instance.clone();
        }
        let target_instance = entry.instance.clone();
        for (key, fm) in &src.fields {
            match parse_field_key(key) {
                Some(id) if t.have.contains(&id) => {
                    let path = retarget(&fm.path, &src.instance, &target_instance);
                    // Nothing was substituted, so this target would publish to
                    // the source's own node. Two devices writing one path is
                    // never what "apply to this article" meant.
                    if path == fm.path {
                        skipped += 1;
                        continue;
                    }
                    entry.fields.insert(
                        key.to_string(),
                        FieldMapping {
                            path,
                            invert: fm.invert,
                        },
                    );
                    copied += 1;
                }
                _ => skipped += 1,
            }
        }
    }
    (copied, skipped)
}

/// Swap one instance segment for another inside a Signal K path.
///
/// Only whole segments are replaced, so an instance that happens to be a
/// substring of a leaf (`house` in `household`) is left alone. An empty source
/// instance means there is nothing to substitute and the path is copied as-is.
fn retarget(path: &str, from: &str, to: &str) -> String {
    if from.is_empty() || from == to {
        return path.to_string();
    }
    path.split('.')
        .map(|seg| if seg == from { to } else { seg })
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod mapping_tests {
    use super::*;

    fn ident(serial: &str, name: &str) -> DeviceIdentity {
        DeviceIdentity {
            article: "66026000".into(),
            serial: serial.into(),
            revision: "A".into(),
            name: name.into(),
            firmware: "2.14".into(),
        }
    }

    fn src_mapping() -> DeviceMapping {
        let mut d = DeviceMapping {
            article: "66026000".into(),
            firmware: "2.14".into(),
            name: "BAT 24V Service".into(),
            instance: "24v-service".into(),
            ..Default::default()
        };
        for (id, leaf) in [
            (0x001u16, "voltage"),
            (0x002, "current"),
            (0x071, "voltage"),
        ] {
            d.fields.insert(
                field_key(id),
                FieldMapping {
                    path: format!("electrical.batteries.24v-service.{leaf}"),
                    invert: false,
                },
            );
        }
        d
    }

    fn target(serial: &str, name: &str, have: &[FieldId]) -> CopyTarget {
        CopyTarget {
            instance: seed::instance_of(name, 0x1000),
            have: have.iter().copied().collect(),
            ident: ident(serial, name),
        }
    }

    #[test]
    fn copying_rewrites_the_instance_segment_per_target() {
        let mut map = Mapping::new();
        let t = target("MLI-2", "BAT 24V Service2", &[0x001, 0x002, 0x071]);
        let (copied, skipped) = copy_to_targets(&mut map, &src_mapping(), &[t]);
        assert_eq!((copied, skipped), (3, 0));
        let d = &map.devices["MLI-2"];
        assert_eq!(d.instance, "24v-service2");
        assert_eq!(
            d.fields[&field_key(0x001)].path,
            "electrical.batteries.24v-service2.voltage"
        );
    }

    /// The cluster case from the bus in #6: two units share an article, but the
    /// master has fields the members do not. Copying must not invent them.
    #[test]
    fn fields_the_target_lacks_are_skipped_not_invented() {
        let mut map = Mapping::new();
        let member = target("MLI-2", "BAT 24V Service2", &[0x001, 0x002]);
        let (copied, skipped) = copy_to_targets(&mut map, &src_mapping(), &[member]);
        assert_eq!((copied, skipped), (2, 1));
        let d = &map.devices["MLI-2"];
        assert!(!d.fields.contains_key(&field_key(0x071)));
    }

    #[test]
    fn copying_records_the_target_identity_not_the_sources() {
        let mut map = Mapping::new();
        let mut t = target("MLI-2", "BAT 24V Service2", &[0x001]);
        t.ident.firmware = "2.15".into();
        copy_to_targets(&mut map, &src_mapping(), &[t]);
        let d = &map.devices["MLI-2"];
        assert_eq!(d.name, "BAT 24V Service2");
        assert_eq!(d.firmware, "2.15");
    }

    /// An instance the user already chose is authoritative; copying must not
    /// silently rename a device's Signal K node underneath them.
    #[test]
    fn an_existing_instance_on_the_target_is_kept() {
        let mut map = Mapping::new();
        map.devices.insert(
            "MLI-2".into(),
            DeviceMapping {
                instance: "port-bank".into(),
                ..Default::default()
            },
        );
        let t = target("MLI-2", "BAT 24V Service2", &[0x001]);
        copy_to_targets(&mut map, &src_mapping(), &[t]);
        let d = &map.devices["MLI-2"];
        assert_eq!(d.instance, "port-bank");
        assert_eq!(
            d.fields[&field_key(0x001)].path,
            "electrical.batteries.port-bank.voltage"
        );
    }

    /// From real use on a live boat: an `INT Nav Chg` was mapped by hand onto
    /// `electrical.chargers.nav-battery`, because that is what it charges. The
    /// device's proposed instance was `nav-chg`, which appears nowhere in that
    /// path. Copying to a sibling would substitute nothing and hand both
    /// devices the same Signal K node.
    #[test]
    fn a_copy_that_would_substitute_nothing_is_skipped() {
        let mut map = Mapping::new();
        let mut src = DeviceMapping {
            article: "77030450".into(),
            instance: "nav-chg".into(),
            ..Default::default()
        };
        src.fields.insert(
            field_key(0x028),
            FieldMapping {
                path: "electrical.chargers.nav-battery.voltage".into(),
                invert: false,
            },
        );
        let t = target("X922S0096", "INT 24V DC/DC", &[0x028]);
        let (copied, skipped) = copy_to_targets(&mut map, &src, &[t]);
        assert_eq!((copied, skipped), (0, 1));
        assert!(map.devices["X922S0096"].fields.is_empty());
    }

    /// With the instance recorded from the path itself, the same copy works.
    #[test]
    fn a_copy_substitutes_the_instance_the_path_actually_uses() {
        let mut map = Mapping::new();
        let mut src = DeviceMapping {
            article: "77030450".into(),
            // What commit_map now records: the segment the path really uses.
            instance: "nav-battery".into(),
            ..Default::default()
        };
        src.fields.insert(
            field_key(0x028),
            FieldMapping {
                path: "electrical.chargers.nav-battery.voltage".into(),
                invert: false,
            },
        );
        let t = target("X922S0096", "INT 24V DC/DC", &[0x028]);
        let (copied, skipped) = copy_to_targets(&mut map, &src, &[t]);
        assert_eq!((copied, skipped), (1, 0));
        assert_eq!(
            map.devices["X922S0096"].fields[&field_key(0x028)].path,
            "electrical.chargers.24v-dc-dc.voltage"
        );
    }

    #[test]
    fn retarget_replaces_whole_segments_only() {
        assert_eq!(
            retarget("electrical.batteries.house.voltage", "house", "port"),
            "electrical.batteries.port.voltage"
        );
        // A leaf that merely contains the instance as a substring is untouched.
        assert_eq!(
            retarget("electrical.batteries.house.household", "house", "port"),
            "electrical.batteries.port.household"
        );
        // Nothing to substitute.
        assert_eq!(retarget("a.b.c", "", "port"), "a.b.c");
        assert_eq!(retarget("a.b.c", "b", "b"), "a.b.c");
    }

    #[test]
    fn the_editor_reports_the_conversion_it_will_apply() {
        let ed = PathEditor {
            field: 0x005,
            field_name: "Temperature".into(),
            unit: "\u{b0}C".into(),
            buf: "electrical.batteries.house.temperature".into(),
            invert: false,
            origin: Origin::Suggested(seed::Tier::Name),
        };
        let hint = ed.conversion_hint().expect("celsius reaches kelvin");
        assert!(hint.contains('K'), "{hint}");

        // Pointing amps at a kelvin leaf has no conversion, and the editor must
        // say so rather than let it be saved.
        let bad = PathEditor {
            unit: "A".into(),
            ..ed
        };
        assert!(bad.conversion_hint().is_none());
    }

    #[test]
    fn an_unknown_leaf_is_allowed_but_flagged_as_unitless() {
        let ed = PathEditor {
            field: 0x005,
            field_name: "Temperature".into(),
            unit: "\u{b0}C".into(),
            buf: "electrical.converters.house.somethingNew".into(),
            invert: false,
            origin: Origin::Blank,
        };
        let hint = ed.conversion_hint().expect("a custom path is allowed");
        assert!(hint.contains("no unit metadata"), "{hint}");
    }
}
