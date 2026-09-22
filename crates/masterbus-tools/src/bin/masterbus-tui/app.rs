//! TUI application state and the logic that mutates it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::path::PathBuf;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use masterbus::{
    AccessLevel, DeviceIdentity, DeviceStatus, FieldId, FieldInfo, GroupInfo, MasterBus, Menu,
    Subscription, Value, VisualizationType, field_id,
};
use masterbus_tools::mapping::{
    CopyTarget, FieldMapping, Mapping, NotifyState, copy_to_targets, field_key,
};
use masterbus_tools::{seed, signalk};

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

/// Which part of a mapping the editor is asking for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Typing the Signal K path.
    Path,
    /// Filling in the truth table for an enum on a boolean leaf; the cursor
    /// is on the label at this index.
    Truth(usize),
    /// Choosing which labels raise a notification; cursor on this index.
    Notify(usize),
}

/// An in-progress edit of one field's Signal K path.
pub struct PathEditor {
    /// Which field is being mapped.
    pub field: FieldId,
    /// Its name, for the modal title.
    pub field_name: String,
    /// Its unit, to derive and display the conversion.
    pub unit: String,
    /// Its labels, if it is an enum; what a truth table is keyed on.
    pub options: Vec<String>,
    /// The path being typed.
    pub buf: String,
    /// Whether to publish the boolean negated.
    pub invert: bool,
    /// Label → boolean, for an enum published to a boolean leaf. Empty until
    /// the path turns out to need one.
    pub truth: BTreeMap<String, bool>,
    /// Label → notification state, for an enum with alarm labels.
    pub notify: BTreeMap<String, NotifyState>,
    /// Where `buf` was seeded from.
    pub origin: Origin,
    /// What the modal is currently asking for.
    pub stage: Stage,
    /// Whether the notification stage has been shown (or skipped) for this
    /// edit, so saving does not loop back into it.
    pub notify_offered: bool,
}

/// What the editor tells the user about the path as typed.
pub enum Hint {
    /// Publishable; describes the unit and conversion, or the truth table.
    Ok(String),
    /// Publishable, but worth a second look.
    Warn(String),
    /// Would be skipped by the sidecar; not saved.
    Refuse(String),
}

impl PathEditor {
    /// What saving the path as typed would do, worked out the way the sidecar
    /// will (see [`signalk::plan`]), so the one moment a human can check that
    /// `°C` becomes kelvin is while choosing the path.
    pub fn plan(&self) -> Result<signalk::Plan, signalk::Refusal> {
        signalk::plan(self.buf.trim(), &self.unit, &self.options, &self.entry())
    }

    /// The mapping entry as it stands.
    pub fn entry(&self) -> FieldMapping {
        FieldMapping {
            path: self.buf.trim().to_string(),
            invert: self.invert,
            truth: self.truth.clone(),
            notify: self.notify.clone(),
            put: false,
        }
    }

    /// Whether to put the notification table in front of the user before
    /// saving: a new mapping of an enum with a label that sounds like
    /// trouble. An existing entry is left as the user last saved it; `^A`
    /// reopens the table on demand.
    pub fn wants_notify_stage(&self) -> bool {
        !self.notify_offered
            && self.origin != Origin::Existing
            && !signalk::notify_default(&self.options).is_empty()
    }

    /// The conversion (or truth table) the current path implies, as a line
    /// for the modal.
    pub fn hint(&self) -> Hint {
        match self.plan() {
            // A three-valued enum is a mode, not a boolean: Standby/On/Alarm
            // squeezed into `enabled` loses Alarm. Say so before the truth
            // table makes the loss look deliberate.
            Err(signalk::Refusal::Truth { .. }) | Ok(_) if self.lossy_boolean() => {
                Hint::Warn(format!(
                    "{} labels → boolean loses info; use {} (string), or Enter for a truth table",
                    self.options.len(),
                    self.mode_leaf()
                ))
            }
            Err(e) => Hint::Refuse(e.to_string()),
            Ok(p) if !p.truth.is_empty() => Hint::Ok(format!(
                "boolean: {}",
                p.truth
                    .iter()
                    .map(|(k, v)| format!("{k}→{v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Ok(p) => match (p.unit, p.warning) {
                (_, Some(w)) => Hint::Warn(w),
                (None, None) if !p.notify.is_empty() => Hint::Ok(format!(
                    "label as-is; notifies on {}",
                    p.notify
                        .iter()
                        .map(|(k, v)| format!("{k} ({})", v.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                (None, None) => Hint::Ok("no unit: published as-is".into()),
                (Some(u), None) => Hint::Ok(format!("→ {u} ({})", p.conv.describe())),
            },
        }
    }

    /// An enum with more than two labels going to a boolean leaf.
    pub fn lossy_boolean(&self) -> bool {
        self.options.len() > 2 && signalk::leaf_is_boolean(self.buf.trim())
    }

    /// The spec's string mode leaf for the path's category, to suggest instead
    /// of a lossy boolean.
    pub fn mode_leaf(&self) -> &'static str {
        signalk::mode_leaf(&self.buf)
    }

    /// Whether the truth table names every label.
    pub fn truth_complete(&self) -> bool {
        self.options.iter().all(|l| self.truth.contains_key(l))
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
        let (buf, invert, truth, notify, origin) = match existing {
            Some(fm) => (fm.path, fm.invert, fm.truth, fm.notify, Origin::Existing),
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
                    Some((s, tier)) => (
                        s.path,
                        s.invert,
                        BTreeMap::new(),
                        BTreeMap::new(),
                        Origin::Suggested(tier),
                    ),
                    None => (
                        self.path_prefix(),
                        false,
                        BTreeMap::new(),
                        BTreeMap::new(),
                        Origin::Blank,
                    ),
                }
            }
        };
        self.path_editor = Some(PathEditor {
            field: field.index,
            field_name: field.name.clone(),
            unit: field.unit.clone(),
            options: field.options.clone(),
            buf,
            invert,
            truth,
            notify,
            origin,
            stage: Stage::Path,
            notify_offered: false,
        });
    }

    /// What to pre-fill when nothing is known about a field: the node of a
    /// path already mapped on this device, so the second field of a solar
    /// charger does not need `electrical.solar.solar-chg.` typed again, else
    /// just `electrical.`.
    fn path_prefix(&self) -> String {
        let node = self
            .mapping
            .as_ref()
            .zip(self.cur_serial.as_ref())
            .and_then(|(s, serial)| s.map.devices.get(serial))
            .and_then(|d| d.fields.values().find_map(|f| signalk::node_of(&f.path)));
        match node {
            Some(n) => format!("{n}."),
            None => "electrical.".into(),
        }
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
        // (a custom path is a legitimate choice, and its unit follows from the
        // device) but a unit pair that cannot be reconciled would be skipped by
        // the sidecar, so it is rejected here where the user can see why. An
        // enum onto a boolean leaf whose labels this build cannot classify
        // needs the user to say which labels mean true: the modal moves on
        // to a truth table instead of saving.
        let plan = match ed.plan() {
            Ok(p) => p,
            Err(signalk::Refusal::Truth { labels }) => {
                let mut ed = ed;
                for l in &labels {
                    if let Some(b) = signalk::truth_of_label(l) {
                        ed.truth.entry(l.clone()).or_insert(b);
                    }
                }
                ed.stage = Stage::Truth(0);
                self.status = if ed.lossy_boolean() {
                    format!(
                        "{} labels → boolean: {} keeps them all; else set true/false per label",
                        ed.options.len(),
                        ed.mode_leaf()
                    )
                } else {
                    "boolean leaf: say which labels mean true (Space toggles, Enter saves)".into()
                };
                self.path_editor = Some(ed);
                return;
            }
            Err(e) => {
                self.status = format!("{e} — not saved");
                self.path_editor = Some(ed);
                return;
            }
        };
        // An enum with a label like `Alarm` is worth a notification, which
        // is what a Signal K server can act on. Offer the table once, with
        // the conventional labels pre-filled, before saving.
        if ed.wants_notify_stage() {
            let mut ed = ed;
            ed.notify = signalk::notify_default(&ed.options);
            ed.notify_offered = true;
            ed.stage = Stage::Notify(0);
            self.status =
                "labels that should raise a Signal K notification (Space cycles, Enter saves)"
                    .into();
            self.path_editor = Some(ed);
            return;
        }
        let identity = self.cur_info.clone();
        let instance = self.cur_instance.clone();
        let Some(s) = self.mapping.as_mut() else {
            return;
        };
        let entry = s.map.devices.entry(serial).or_default();
        let existing_put = entry
            .fields
            .get(&field_key(ed.field))
            .is_some_and(|f| f.put);
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
        // Record the truth table the sidecar will use, even when it was
        // derived from the conventional label meanings, so the file says what
        // it does and a future build cannot silently change its mind.
        entry.fields.insert(
            field_key(ed.field),
            FieldMapping {
                path: path.clone(),
                invert: ed.invert,
                truth: plan.truth,
                notify: plan.notify.clone(),
                // The TUI does not edit this flag; an existing setting survives
                // a re-map of the same field.
                put: existing_put,
            },
        );
        s.dirty = true;
        self.status = if plan.notify.is_empty() {
            format!("{} → {path}", field_key(ed.field))
        } else {
            format!(
                "{} → {path}, notifying on {}",
                field_key(ed.field),
                plan.notify
                    .iter()
                    .map(|(k, v)| format!("{k} ({})", v.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
    }

    /// Whether the editor is on the notification stage.
    pub fn notify_editing(&self) -> bool {
        matches!(
            self.path_editor.as_ref().map(|e| e.stage),
            Some(Stage::Notify(_))
        )
    }

    /// `^A` in the path stage: open the notification table for an enum,
    /// whatever its labels, pre-filled from the conventions if it is empty.
    pub fn open_notify(&mut self) {
        let Some(ed) = self.path_editor.as_mut() else {
            return;
        };
        if ed.options.is_empty() {
            self.status = "only an enum (a field with labels) can notify".into();
            return;
        }
        if ed.notify.is_empty() {
            ed.notify = signalk::notify_default(&ed.options);
        }
        ed.notify_offered = true;
        ed.stage = Stage::Notify(0);
    }

    /// Move the notification cursor.
    pub fn notify_move(&mut self, delta: i32) {
        if let Some(ed) = self.path_editor.as_mut()
            && let Stage::Notify(i) = ed.stage
            && !ed.options.is_empty()
        {
            let n = ed.options.len() as i32;
            ed.stage = Stage::Notify((i as i32 + delta).rem_euclid(n) as usize);
        }
    }

    /// Cycle the selected label: normal → alert → warn → alarm → emergency
    /// → normal.
    pub fn notify_cycle(&mut self) {
        if let Some(ed) = self.path_editor.as_mut()
            && let Stage::Notify(i) = ed.stage
            && let Some(label) = ed.options.get(i).cloned()
        {
            let next = match ed.notify.get(&label) {
                None => Some(NotifyState::ALL[0]),
                Some(s) => NotifyState::ALL
                    .iter()
                    .position(|x| x == s)
                    .and_then(|p| NotifyState::ALL.get(p + 1))
                    .copied(),
            };
            match next {
                Some(s) => {
                    ed.notify.insert(label, s);
                }
                None => {
                    ed.notify.remove(&label);
                }
            }
        }
    }

    /// Set the selected label outright; `None` means normal.
    pub fn notify_set(&mut self, state: Option<NotifyState>) {
        if let Some(ed) = self.path_editor.as_mut()
            && let Stage::Notify(i) = ed.stage
            && let Some(label) = ed.options.get(i).cloned()
        {
            match state {
                Some(s) => {
                    ed.notify.insert(label, s);
                }
                None => {
                    ed.notify.remove(&label);
                }
            }
        }
    }

    /// Leave the notification table for the path, keeping what was set.
    pub fn notify_back(&mut self) {
        if let Some(ed) = self.path_editor.as_mut() {
            ed.stage = Stage::Path;
        }
    }

    /// Save from the notification stage.
    pub fn commit_notify(&mut self) {
        if let Some(ed) = self.path_editor.as_mut() {
            ed.notify_offered = true;
        }
        self.commit_map();
    }

    /// Whether the editor is on the truth-table stage.
    pub fn truth_editing(&self) -> bool {
        matches!(
            self.path_editor.as_ref().map(|e| e.stage),
            Some(Stage::Truth(_))
        )
    }

    /// Move the truth-table cursor.
    pub fn truth_move(&mut self, delta: i32) {
        if let Some(ed) = self.path_editor.as_mut()
            && let Stage::Truth(i) = ed.stage
            && !ed.options.is_empty()
        {
            let n = ed.options.len() as i32;
            ed.stage = Stage::Truth((i as i32 + delta).rem_euclid(n) as usize);
        }
    }

    /// Cycle the selected label: unset → true → false → true …
    pub fn truth_toggle(&mut self) {
        if let Some(ed) = self.path_editor.as_mut()
            && let Stage::Truth(i) = ed.stage
            && let Some(label) = ed.options.get(i).cloned()
        {
            let next = !ed.truth.get(&label).copied().unwrap_or(false);
            ed.truth.insert(label, next);
        }
    }

    /// Set the selected label outright.
    pub fn truth_set(&mut self, value: bool) {
        if let Some(ed) = self.path_editor.as_mut()
            && let Stage::Truth(i) = ed.stage
            && let Some(label) = ed.options.get(i).cloned()
        {
            ed.truth.insert(label, value);
        }
    }

    /// Leave the truth table for the path, keeping what was filled in.
    pub fn truth_back(&mut self) {
        if let Some(ed) = self.path_editor.as_mut() {
            ed.stage = Stage::Path;
        }
    }

    /// Save from the truth-table stage: every label needs a value first.
    pub fn commit_truth(&mut self) {
        let Some(ed) = self.path_editor.as_ref() else {
            return;
        };
        if !ed.truth_complete() {
            self.status = "every label needs true or false before saving".into();
            return;
        }
        self.commit_map();
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

    /// `^N` in the editor. For a real boolean field this toggles `invert`.
    /// For an enum on a boolean leaf it flips the truth table instead, so
    /// what the user sees is `Standby→true, Activated→false` rather than a
    /// separate "inverted" flag they have to apply in their head.
    pub fn map_editor_toggle_invert(&mut self) {
        let Some(ed) = self.path_editor.as_mut() else {
            return;
        };
        if ed.options.is_empty() || !signalk::leaf_is_boolean(ed.buf.trim()) {
            ed.invert = !ed.invert;
            return;
        }
        // Materialise the table the sidecar would use, then flip it.
        if let Ok(plan) = ed.plan()
            && !plan.truth.is_empty()
        {
            ed.truth = plan.truth;
        } else {
            for l in &ed.options {
                if let Some(b) = signalk::truth_of_label(l) {
                    ed.truth.entry(l.clone()).or_insert(b);
                }
            }
        }
        for v in ed.truth.values_mut() {
            *v = !*v;
        }
        ed.invert = false;
    }

    /// Whether `^N` would flip a truth table rather than the invert flag.
    pub fn flips_truth(&self) -> bool {
        self.path_editor
            .as_ref()
            .is_some_and(|ed| !ed.options.is_empty() && signalk::leaf_is_boolean(ed.buf.trim()))
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

#[cfg(test)]
mod mapping_tests {
    use super::*;

    fn editor(unit: &str, buf: &str) -> PathEditor {
        PathEditor {
            field: 0x005,
            field_name: "Temperature".into(),
            unit: unit.into(),
            options: Vec::new(),
            buf: buf.into(),
            invert: false,
            truth: BTreeMap::new(),
            notify: BTreeMap::new(),
            origin: Origin::Blank,
            stage: Stage::Path,
            notify_offered: false,
        }
    }

    #[test]
    fn the_notification_stage_is_offered_once_for_alarm_labels() {
        let mut ed = editor("", "electrical.inverters.inv.inverterMode");
        ed.options = vec!["Standby".into(), "On".into(), "Alarm".into()];
        assert!(ed.wants_notify_stage());
        ed.notify_offered = true;
        assert!(!ed.wants_notify_stage());
        // An existing entry is left as saved.
        let mut ed = editor("", "electrical.inverters.inv.inverterMode");
        ed.options = vec!["Standby".into(), "On".into(), "Alarm".into()];
        ed.origin = Origin::Existing;
        assert!(!ed.wants_notify_stage());
        // Ordinary labels never prompt.
        let mut ed = editor("", "electrical.chargers.c.chargingMode");
        ed.options = vec!["Off".into(), "Bulk".into(), "Float".into()];
        assert!(!ed.wants_notify_stage());
    }

    #[test]
    fn the_hint_names_the_notifying_labels() {
        let mut ed = editor("", "electrical.inverters.inv.inverterMode");
        ed.options = vec!["Standby".into(), "On".into(), "Alarm".into()];
        ed.notify = [("Alarm".to_string(), NotifyState::Alarm)].into();
        let Hint::Ok(h) = ed.hint() else {
            panic!("a mode leaf with a notification is fine")
        };
        assert!(h.contains("Alarm (alarm)"), "{h}");
    }

    #[test]
    fn the_editor_reports_the_conversion_it_will_apply() {
        let ed = editor("\u{b0}C", "electrical.batteries.house.temperature");
        let Hint::Ok(hint) = ed.hint() else {
            panic!("celsius reaches kelvin")
        };
        assert!(hint.contains("→ K"), "{hint}");

        // Pointing amps at a kelvin leaf has no conversion, and the editor must
        // say so rather than let it be saved.
        let bad = editor("A", "electrical.batteries.house.temperature");
        assert!(matches!(bad.hint(), Hint::Refuse(_)));
    }

    /// A custom or newer-spec leaf is allowed, and its unit still follows
    /// from the device, so the editor shows the conversion rather than a
    /// warning about missing metadata.
    #[test]
    fn an_unknown_leaf_still_shows_the_devices_unit() {
        let ed = editor("\u{b0}C", "electrical.converters.house.somethingNew");
        let Hint::Ok(hint) = ed.hint() else {
            panic!("a custom path is allowed")
        };
        assert!(hint.contains("→ K"), "{hint}");
        // Only a unit this build cannot convert is worth a warning.
        let odd = editor("l/h", "electrical.converters.house.flow");
        assert!(matches!(odd.hint(), Hint::Warn(_)));
    }

    #[test]
    fn an_enum_on_a_boolean_leaf_shows_its_truth_table() {
        let mut ed = editor("", "electrical.switches.out.state");
        ed.options = vec!["Standby".into(), "Activated".into()];
        let Hint::Ok(hint) = ed.hint() else {
            panic!("conventional labels need no help")
        };
        assert!(hint.contains("Activated→true"), "{hint}");
        // A third label makes it a mode, not a boolean: the editor steers
        // towards the string leaf before offering the truth table, and keeps
        // saying so once the table is filled in.
        ed.options.push("Alarm".into());
        assert!(matches!(ed.hint(), Hint::Warn(_)));
        assert!(!ed.truth_complete());
        ed.truth = [("Standby", false), ("Activated", true), ("Alarm", false)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert!(ed.truth_complete());
        assert!(matches!(ed.hint(), Hint::Warn(_)));
    }

    /// `^N` on an enum flips the table the user is looking at, rather than
    /// recording an invert flag they would have to apply in their head.
    #[test]
    fn invert_on_an_enum_flips_the_truth_table() {
        let mut ed = editor("", "electrical.switches.out.state");
        ed.options = vec!["Standby".into(), "Activated".into()];
        let mut app_ed = Some(ed);
        // Drive the same logic the App method uses, on a bare editor.
        let ed = app_ed.as_mut().unwrap();
        let plan = ed.plan().unwrap();
        ed.truth = plan.truth;
        for v in ed.truth.values_mut() {
            *v = !*v;
        }
        assert!(ed.truth["Standby"]);
        assert!(!ed.truth["Activated"]);
        assert!(!ed.invert);
        let Hint::Ok(h) = ed.hint() else {
            panic!("a complete table is fine")
        };
        assert!(h.contains("Standby→true"), "{h}");
    }

    #[test]
    fn the_mode_leaf_suggested_follows_the_category() {
        let mut ed = editor("", "electrical.inverters.inv.enabled");
        ed.options = vec!["Standby".into(), "On".into(), "Alarm".into()];
        assert!(ed.lossy_boolean());
        assert_eq!(ed.mode_leaf(), "inverterMode");
        let Hint::Warn(w) = ed.hint() else {
            panic!("three labels onto enabled should warn")
        };
        assert!(w.contains("inverterMode"), "{w}");
        ed.buf = "electrical.chargers.chg.enabled".into();
        assert_eq!(ed.mode_leaf(), "chargingMode");
        // Two labels is a genuine boolean; nothing to steer away from.
        ed.options.pop();
        assert!(!ed.lossy_boolean());
    }
}

/// Navigation, tab and editor tests, driven against a loopback bus.
///
/// [`App`] owns a real [`MasterBus`], so these build one over a fake
/// transport: a device that announces itself and answers Btm1 value reads and
/// writes. Rows normally arrive from a discovery worker thread; a test seeds
/// `rows` and `values` directly (both are `pub`) so the state machine under
/// test is the navigation and editing logic, not discovery.
#[cfg(test)]
pub(crate) mod app_tests {
    use super::*;
    use masterbus::Config;
    use masterbus::transport::{Transport, TransportRx, TransportTx};
    use std::collections::HashMap as Map;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(crate) const ADDR: u32 = 0x188EA2;

    // ── a loopback bus with one device ──────────────────────────────────────

    type Frame = (u32, Vec<u8>);

    /// Shared device state: the Btm1 field values the fake answers with.
    #[derive(Default)]
    struct DeviceState {
        btm1: Map<u8, [u8; 4]>,
    }

    /// The device's one Monitoring group, as `(wire index, viz code, option
    /// count, writable)`. Discovery reads this, so the engine ends up knowing
    /// the same fields the tests put in `App::rows` — which is what lets a
    /// write actually reach the wire.
    const FIELDS: [(u8, u8, f32, bool); 6] = [
        (0x01, 0x06, 0.0, true),  // Device name, Text
        (0x05, 0x03, 3.0, true),  // Mode, DropDown with 3 options
        (0x13, 0x05, 0.0, true),  // Inverter, CheckBox
        (0x17, 0x01, 0.0, true),  // Voltage, Float
        (0x18, 0x01, 0.0, false), // Current, Float, read-only
        (0x19, 0x01, 0.0, true),  // Frequency, Float
    ];

    struct FakeTransport {
        up: crossbeam_channel::Receiver<Frame>,
        down: crossbeam_channel::Sender<Frame>,
    }

    impl Transport for FakeTransport {
        fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
            (Box::new(FakeRx(self.up)), Box::new(FakeTx(self.down)))
        }
    }

    struct FakeRx(crossbeam_channel::Receiver<Frame>);

    impl TransportRx for FakeRx {
        fn recv(&mut self, timeout: Duration) -> masterbus::Result<Option<Frame>> {
            match self.0.recv_timeout(timeout) {
                Ok(f) => Ok(Some(f)),
                // A stopped device thread idles the reader rather than
                // spinning it.
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
                Err(_) => {
                    std::thread::sleep(timeout);
                    Ok(None)
                }
            }
        }
    }

    struct FakeTx(crossbeam_channel::Sender<Frame>);

    impl TransportTx for FakeTx {
        fn send(&mut self, can_id: u32, data: &[u8]) -> masterbus::Result<()> {
            let _ = self.0.send((can_id, data.to_vec()));
            Ok(())
        }
    }

    /// Keeps the device thread alive for the lifetime of a test.
    pub(crate) struct Bus {
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<DeviceState>>,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Answer one frame. Strings are all reported as string id 0 ("no
    /// string"), so discovery never pays for a chunk fetch — the tests care
    /// about types and writability, not names.
    fn respond(st: &mut DeviceState, can_id: u32, data: &[u8]) -> Vec<Frame> {
        const SHADOW: u32 = 0x80_0000;
        let class = ((can_id >> 24) & 0x1F) as u8;
        let addr = can_id & 0x00FF_FFFF;
        if addr & !SHADOW != ADDR {
            return Vec::new();
        }
        let shadow = addr & SHADOW != 0;
        match (class, shadow, data) {
            // Group count per menu selector: one Monitoring group, nothing else.
            (0x07, false, [0x08, sel]) => {
                let n: u16 = if *sel == 0x02 { 1 } else { 0 };
                let [lo, hi] = n.to_le_bytes();
                vec![(0x06_000000 | ADDR, vec![0x08, *sel, lo, hi])]
            }
            // Monitoring schema, group 0.
            (0x19, false, [0x28, 0, _]) => {
                vec![(0x09_000000 | ADDR, vec![0x28, 0, 0, 0, 0, 0])]
            }
            (0x19, false, [0x07, 0, _]) => {
                let mut d = vec![0x07, 0, 0, 0];
                d.extend_from_slice(&(FIELDS.len() as f32).to_le_bytes());
                vec![(0x09_000000 | ADDR, d)]
            }
            (0x19, false, [0x03, 0, _, idx]) => match FIELDS.get(*idx as usize) {
                Some((wire, ..)) => vec![(0x09_000000 | ADDR, vec![0x03, 0, 0, *idx, *wire, 0])],
                None => Vec::new(),
            },
            // Per-field metadata on the shadow address.
            (0x18, true, [0x26, _, _, _]) => {
                // Option string id 0 — an unnamed option, no chunk fetch.
                let mut d = data.to_vec();
                d.extend_from_slice(&[0, 0]);
                vec![(0x08_000000 | ADDR | SHADOW, d)]
            }
            (0x18, true, [op, lo, hi]) => {
                let wire = u16::from_le_bytes([*lo, *hi]);
                let Some(&(_, viz, max, writeable)) =
                    FIELDS.iter().find(|(w, ..)| *w as u16 == wire)
                else {
                    return Vec::new();
                };
                let mut d = vec![*op, *lo, *hi, 0];
                match *op {
                    0x28 | 0x2C => d.extend_from_slice(&[0, 0]), // name / unit: no string
                    0x02 => d.push(viz),
                    0x07 => d.extend_from_slice(&max.to_le_bytes()),
                    0x0B => d.push(writeable as u8),
                    _ => return Vec::new(), // includes the eventable probe
                }
                vec![(0x08_000000 | ADDR | SHADOW, d)]
            }
            // Btm1 values: two bytes reads, six bytes writes.
            (0x18, false, [f, tab]) => {
                let v = st.btm1.get(f).copied().unwrap_or_default();
                let mut d = vec![*f, *tab];
                d.extend_from_slice(&v);
                vec![(0x08_000000 | ADDR, d)]
            }
            (0x18, false, [f, _, a, b, c, e]) => {
                st.btm1.insert(*f, [*a, *b, *c, *e]);
                Vec::new()
            }
            // String-chunk write: ack by echoing the header.
            (0x07, false, [0x30, lo, hi, seq, ..]) if data.len() >= 5 => {
                vec![(0x06_000000 | ADDR, vec![0x30, *lo, *hi, *seq])]
            }
            _ => Vec::new(),
        }
    }

    /// Start the device thread and connect a `MasterBus` to it.
    pub(crate) fn bus() -> (MasterBus, Bus) {
        let (down_tx, down_rx) = crossbeam_channel::unbounded::<Frame>();
        let (up_tx, up_rx) = crossbeam_channel::unbounded::<Frame>();
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(DeviceState::default()));

        {
            let (stop, state) = (stop.clone(), state.clone());
            std::thread::spawn(move || {
                let mut next = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    if Instant::now() >= next {
                        // class 0x04 self-announcement
                        if up_tx
                            .send((0x04_000000 | ADDR, vec![0x0B, 0, 0, 0, 0x02, 0x01]))
                            .is_err()
                        {
                            return;
                        }
                        next = Instant::now() + Duration::from_millis(10);
                    }
                    match down_rx.recv_timeout(Duration::from_millis(1)) {
                        Ok((id, data)) => {
                            let replies = respond(&mut state.lock().unwrap(), id, &data);
                            for r in replies {
                                if up_tx.send(r).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(_) => return,
                    }
                }
            });
        }

        let config = Config {
            min_send_interval: Duration::ZERO,
            discovery_timeout: Duration::from_millis(5),
            discovery_retries: 1,
            discovery_window: Duration::ZERO,
            discovery_settle: Duration::ZERO,
            connect_timeout: Duration::from_secs(5),
            cache_path: None,
            ..Default::default()
        };
        let transport = Box::new(FakeTransport {
            up: up_rx,
            down: down_tx,
        });
        let bus = MasterBus::with_transport(transport, config).expect("connect");
        (bus, Bus { stop, state })
    }

    pub(crate) fn app() -> (App, Bus) {
        let (bus, guard) = bus();
        let app = App::new(
            bus,
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            None,
            false,
        );
        (app, guard)
    }

    pub(crate) fn field(
        index: FieldId,
        name: &str,
        viz: VisualizationType,
        writeable: bool,
    ) -> FieldInfo {
        FieldInfo {
            index,
            name: name.to_string(),
            unit: "V".into(),
            viz_type: viz,
            writeable,
            eventable: false,
            min: 0.0,
            max: 0.0,
            step: 0.0,
            options: Vec::new(),
        }
    }

    /// Pump the discovery worker until its result lands, the way the event
    /// loop does between key presses.
    pub(crate) fn settle(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.discovering() && Instant::now() < deadline {
            app.poll_pending();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.discovering(), "discovery never finished");
    }

    /// A group header followed by two fields, which is what the row pane
    /// looks like after a menu is discovered.
    fn seed_rows(app: &mut App) {
        app.cur_device = Some(ADDR);
        app.rows = vec![
            Row::Group("DC".into()),
            Row::Field(field(
                field_id::btm1(0x17),
                "Voltage",
                VisualizationType::Float,
                true,
            )),
            Row::Field(field(
                field_id::btm1(0x18),
                "Current",
                VisualizationType::Float,
                false,
            )),
        ];
        app.row_sel = 1;
    }

    // ── device pane ─────────────────────────────────────────────────────────

    #[test]
    fn a_new_app_starts_on_the_device_list() {
        let (app, _bus) = app();
        assert_eq!(app.device_ids, vec![ADDR]);
        assert!(app.focus == Focus::Devices);
        assert!(app.cur_device.is_none());
        assert!(!app.should_quit);
        assert_eq!(app.device_status(ADDR), DeviceStatus::On);
    }

    /// A device with no name yet is labelled by its id, so the list is never
    /// blank while the backfill thread is still working.
    #[test]
    fn a_device_without_a_name_falls_back_to_its_id() {
        let (app, _bus) = app();
        assert_eq!(app.device_label(ADDR), ADDR.to_string());
        app.names.lock().unwrap().insert(ADDR, "Combi".into());
        assert_eq!(app.device_label(ADDR), "Combi");
    }

    /// Selection clamps at both ends rather than wrapping — the device list
    /// is a list, not a carousel.
    #[test]
    fn device_selection_clamps_at_both_ends() {
        let (mut app, _bus) = app();
        app.note_alive(0x3A3B4B);
        app.note_alive(0x43DF24);
        assert_eq!(app.device_ids.len(), 3);

        app.move_device(-1);
        assert_eq!(app.dev_sel, 0);
        app.move_device(1);
        assert_eq!(app.dev_sel, 1);
        app.move_device(10);
        assert_eq!(app.dev_sel, 2);
        app.move_device(-10);
        assert_eq!(app.dev_sel, 0);
    }

    /// A device already in the list isn't added twice when it re-announces.
    #[test]
    fn note_alive_does_not_duplicate_a_known_device() {
        let (mut app, _bus) = app();
        app.note_alive(ADDR);
        app.note_alive(ADDR);
        assert_eq!(app.device_ids, vec![ADDR]);
    }

    #[test]
    fn moving_within_an_empty_device_list_is_a_no_op() {
        let (mut app, _bus) = app();
        app.device_ids.clear();
        app.move_device(1);
        assert_eq!(app.dev_sel, 0);
    }

    /// Opening lands on Summary with the field pane focused, and going back
    /// clears everything the device left behind.
    #[test]
    fn opening_a_device_lands_on_summary_and_back_clears_it() {
        let (mut app, _bus) = app();
        app.open_device();

        assert_eq!(app.cur_device, Some(ADDR));
        assert!(app.focus == Focus::Fields);
        assert!(app.cur_tab == TabKind::Summary);

        app.rows.push(Row::Group("stale".into()));
        app.loaded_menus.insert(Menu::Monitoring);
        app.settings_loaded = true;

        app.back_to_devices();
        assert!(app.focus == Focus::Devices);
        assert!(app.cur_device.is_none());
        assert!(app.rows.is_empty());
        assert!(app.loaded_menus.is_empty());
        assert!(!app.settings_loaded);
        assert!(app.cur_info.is_none());
    }

    #[test]
    fn opening_with_no_devices_is_a_no_op() {
        let (mut app, _bus) = app();
        app.device_ids.clear();
        app.open_device();
        assert!(app.cur_device.is_none());
    }

    // ── tabs ────────────────────────────────────────────────────────────────

    /// Tabs wrap in both directions, and Summary is always position 0. Each
    /// data tab starts its own discovery, which parks cycling until it
    /// finishes or is cancelled — so the walk cancels as it goes.
    #[test]
    fn tabs_cycle_in_both_directions() {
        let (mut app, _bus) = app();
        app.open_device();
        assert!(app.cur_tab == TabKind::Summary);

        for expected in &TABS[1..] {
            app.next_tab();
            assert!(app.cur_tab == *expected);
            assert!(app.discovering(), "{expected:?} should start a discovery");
            settle(&mut app);
        }

        // Past the last tab, round to Summary — which needs no discovery.
        app.next_tab();
        assert!(app.cur_tab == TabKind::Summary);
        assert!(!app.discovering());

        // And backwards from Summary lands on the last tab.
        app.prev_tab();
        assert!(app.cur_tab == TABS[TABS.len() - 1]);
    }

    /// Nothing cycles until a device is open, and nothing cycles while a
    /// discovery is in flight — the worker's result would land on the wrong
    /// tab.
    #[test]
    fn tabs_do_not_cycle_without_a_device_or_during_discovery() {
        let (mut app, _bus) = app();
        app.next_tab();
        assert!(app.cur_tab == TabKind::Summary);

        app.open_device();
        app.next_tab();
        let parked = app.cur_tab;
        let (_tx, rx) = bounded(1);
        app.pending = Some(Pending {
            id: ADDR,
            tab: TabKind::Settings,
            name: "Settings".into(),
            started: Instant::now(),
            rx,
        });
        app.next_tab();
        assert!(app.cur_tab == parked);
        assert!(app.discovering());
    }

    /// Cancelling abandons the discovery *and* the device: there is nothing
    /// sensible to show on a half-discovered tab.
    #[test]
    fn cancelling_a_discovery_returns_to_the_device_list() {
        let (mut app, _bus) = app();
        app.open_device();
        let (_tx, rx) = bounded(1);
        app.pending = Some(Pending {
            id: ADDR,
            tab: TabKind::Settings,
            name: "Settings".into(),
            started: Instant::now(),
            rx,
        });
        let (name, tab, _secs) = app.pending_info().unwrap();
        assert_eq!(name, "Settings");
        assert!(tab == TabKind::Settings);

        app.cancel_pending();
        assert!(!app.discovering());
        assert!(app.pending_info().is_none());
        assert!(app.cur_device.is_none());
        assert!(app.focus == Focus::Devices);
        assert_eq!(app.status, "discovery cancelled");
    }

    /// The whole discovery round trip: switching to a menu spawns a worker,
    /// polling picks up its result, and the rows appear with the group
    /// header the device reported.
    #[test]
    fn discovering_a_menu_fills_the_row_pane() {
        let (mut app, _bus) = app();
        app.open_device();
        app.next_tab();
        assert!(app.cur_tab == TABS[1]);
        settle(&mut app);

        // One group header plus every field the device enumerated.
        assert_eq!(app.rows.len(), 1 + FIELDS.len());
        assert!(matches!(app.rows[0], Row::Group(_)));
        let indices: Vec<FieldId> = app
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Field(f) => Some(f.index),
                _ => None,
            })
            .collect();
        assert_eq!(
            indices,
            FIELDS
                .iter()
                .map(|(w, ..)| field_id::btm1(*w))
                .collect::<Vec<_>>()
        );
        assert!(app.loaded_menus.contains(&Menu::Monitoring));
        // The first field is selected and its value has been read.
        assert!(matches!(app.rows[app.row_sel], Row::Field(_)));
    }

    /// A tab already discovered is rebuilt from the library's cache — no
    /// second worker, no second sweep of the bus.
    #[test]
    fn revisiting_a_discovered_tab_needs_no_second_discovery() {
        let (mut app, _bus) = app();
        app.open_device();
        app.next_tab();
        settle(&mut app);
        let rows = app.rows.len();

        app.prev_tab(); // back to Summary
        assert!(app.cur_tab == TabKind::Summary);
        app.next_tab(); // and into Monitoring again
        assert!(!app.discovering(), "the menu is already loaded");
        assert_eq!(app.rows.len(), rows);
    }

    /// The Settings tab is the Btm3 flat probe; a Btm1-only device has
    /// nothing to show there, which is not an error.
    #[test]
    fn the_settings_tab_is_empty_on_a_btm1_only_device() {
        let (mut app, _bus) = app();
        app.open_device();
        app.prev_tab(); // Summary → the last tab, Settings
        assert!(app.cur_tab == TabKind::Settings);
        settle(&mut app);

        assert!(app.settings_loaded);
        assert!(app.rows.is_empty());
    }

    // ── row pane ────────────────────────────────────────────────────────────

    /// Row movement skips group headers: only fields are selectable.
    #[test]
    fn row_movement_skips_group_headers() {
        let (mut app, _bus) = app();
        app.cur_device = Some(ADDR);
        app.rows = vec![
            Row::Group("DC".into()),
            Row::Field(field(
                field_id::btm1(0x17),
                "Voltage",
                VisualizationType::Float,
                true,
            )),
            Row::Group("AC".into()),
            Row::Field(field(
                field_id::btm1(0x19),
                "Frequency",
                VisualizationType::Float,
                true,
            )),
        ];
        app.row_sel = 1;

        app.move_row(1);
        assert_eq!(app.row_sel, 3, "should have jumped over the AC header");
        app.move_row(-1);
        assert_eq!(app.row_sel, 1);
    }

    /// At the edges the selection stays put rather than wrapping onto a
    /// header or off the end.
    #[test]
    fn row_movement_stops_at_the_edges() {
        let (mut app, _bus) = app();
        seed_rows(&mut app);

        app.move_row(-1);
        assert_eq!(app.row_sel, 1, "no field above the first one");
        app.move_row(1);
        assert_eq!(app.row_sel, 2);
        app.move_row(1);
        assert_eq!(app.row_sel, 2, "no field below the last one");
    }

    #[test]
    fn moving_within_an_empty_row_pane_is_a_no_op() {
        let (mut app, _bus) = app();
        app.move_row(1);
        assert_eq!(app.row_sel, 0);
    }

    // ── editing ─────────────────────────────────────────────────────────────

    /// A read-only field says so instead of opening an editor.
    #[test]
    fn a_read_only_field_refuses_to_open_an_editor() {
        let (mut app, _bus) = app();
        seed_rows(&mut app);
        app.row_sel = 2; // Current, writeable: false

        app.begin_edit();
        assert!(!app.editing());
        assert_eq!(app.status, "Current is read-only");
    }

    /// A numeric editor pre-fills with the cached value, takes only numeric
    /// characters, and writes the parsed result through to the device.
    #[test]
    fn a_numeric_edit_pre_fills_filters_and_commits() {
        let (mut app, bus) = app();
        seed_rows(&mut app);
        app.values.insert(field_id::btm1(0x17), Value::Float(12.5));

        app.begin_edit();
        let Some(Editor {
            kind: EditKind::Number(buf),
            ..
        }) = &app.editor
        else {
            panic!("expected a numeric editor");
        };
        assert_eq!(buf, "12.5");

        app.editor_backspace();
        app.editor_backspace();
        app.editor_backspace();
        app.editor_backspace();
        for c in "13.2xyz".chars() {
            app.editor_char(c);
        }
        let Some(Editor {
            kind: EditKind::Number(buf),
            ..
        }) = &app.editor
        else {
            panic!("expected a numeric editor");
        };
        assert_eq!(buf, "13.2", "letters must not reach the buffer");

        app.commit_edit();
        assert!(!app.editing());
        assert_eq!(app.status, "set ok");
        assert_eq!(bus.state.lock().unwrap().btm1[&0x17], 13.2f32.to_le_bytes());
    }

    /// A number that doesn't parse reports itself and writes nothing.
    #[test]
    fn a_malformed_number_is_reported_and_not_written() {
        let (mut app, bus) = app();
        seed_rows(&mut app);

        app.begin_edit();
        app.editor_char('-');
        app.commit_edit();

        assert!(!app.editing());
        assert_eq!(app.status, "'-' is not a number");
        assert!(bus.state.lock().unwrap().btm1.is_empty());
    }

    #[test]
    fn cancelling_an_edit_discards_it() {
        let (mut app, bus) = app();
        seed_rows(&mut app);

        app.begin_edit();
        app.editor_char('9');
        app.cancel_edit();

        assert!(!app.editing());
        assert_eq!(app.status, "edit cancelled");
        app.commit_edit(); // nothing staged — must not panic or write
        assert!(bus.state.lock().unwrap().btm1.is_empty());
    }

    /// A boolean field has no editor: the key press toggles it straight away
    /// against the cached value.
    #[test]
    fn a_boolean_field_toggles_without_an_editor() {
        let (mut app, bus) = app();
        app.cur_device = Some(ADDR);
        app.rows = vec![Row::Field(field(
            field_id::btm1(0x13),
            "Inverter",
            VisualizationType::CheckBox,
            true,
        ))];
        app.row_sel = 0;
        app.values
            .insert(field_id::btm1(0x13), Value::Boolean(false));

        app.begin_edit();
        assert!(!app.editing(), "a checkbox writes immediately");
        assert_eq!(bus.state.lock().unwrap().btm1[&0x13], 1.0f32.to_le_bytes());
    }

    /// The choice editor starts on the current selection and wraps.
    #[test]
    fn a_choice_edit_starts_on_the_current_option_and_wraps() {
        let (mut app, bus) = app();
        app.cur_device = Some(ADDR);
        let mut f = field(
            field_id::btm1(0x05),
            "Mode",
            VisualizationType::DropDown,
            true,
        );
        f.options = vec!["Off".into(), "On".into(), "Auto".into()];
        app.rows = vec![Row::Field(f)];
        app.row_sel = 0;
        app.values.insert(
            field_id::btm1(0x05),
            Value::List {
                index: 1,
                options: vec![],
            },
        );

        app.begin_edit();
        let Some(Editor {
            kind: EditKind::Choice { sel, .. },
            ..
        }) = &app.editor
        else {
            panic!("expected a choice editor");
        };
        assert_eq!(*sel, 1);

        app.editor_choice_move(1);
        app.editor_choice_move(1); // past the end → wraps to 0
        let Some(Editor {
            kind: EditKind::Choice { sel, .. },
            ..
        }) = &app.editor
        else {
            panic!("expected a choice editor");
        };
        assert_eq!(*sel, 0);

        app.editor_choice_move(-1); // before the start → wraps to the last
        let Some(Editor {
            kind: EditKind::Choice { sel, .. },
            ..
        }) = &app.editor
        else {
            panic!("expected a choice editor");
        };
        assert_eq!(*sel, 2);

        app.commit_edit();
        assert_eq!(bus.state.lock().unwrap().btm1[&0x05], 2.0f32.to_le_bytes());
    }

    /// A stale cached index past the end of the option list must not panic
    /// the editor.
    #[test]
    fn a_choice_index_past_the_options_is_clamped() {
        let (mut app, _bus) = app();
        app.cur_device = Some(ADDR);
        let mut f = field(
            field_id::btm1(0x05),
            "Mode",
            VisualizationType::DropDown,
            true,
        );
        f.options = vec!["Off".into(), "On".into()];
        app.rows = vec![Row::Field(f)];
        app.row_sel = 0;
        app.values.insert(
            field_id::btm1(0x05),
            Value::List {
                index: 99,
                options: vec![],
            },
        );

        app.begin_edit();
        let Some(Editor {
            kind: EditKind::Choice { sel, .. },
            ..
        }) = &app.editor
        else {
            panic!("expected a choice editor");
        };
        assert_eq!(*sel, 1);
    }

    /// Text edits are capped at the wire limit while typing, so the user
    /// can't compose a string the device would reject.
    #[test]
    fn a_text_edit_is_capped_at_the_wire_limit() {
        let (mut app, _bus) = app();
        app.cur_device = Some(ADDR);
        app.rows = vec![Row::Field(field(
            field_id::btm1(0x01),
            "Device name",
            VisualizationType::Text,
            true,
        ))];
        app.row_sel = 0;
        app.values.insert(
            field_id::btm1(0x01),
            Value::Text {
                sid: 1,
                text: "Combi".into(),
            },
        );

        app.begin_edit();
        let Some(Editor {
            kind: EditKind::Text { str_id, buf },
            ..
        }) = &app.editor
        else {
            panic!("expected a text editor");
        };
        assert_eq!((*str_id, buf.as_str()), (1, "Combi"));

        for c in "0123456789012345678901234567890".chars() {
            app.editor_char(c);
        }
        let Some(Editor {
            kind: EditKind::Text { buf, .. },
            ..
        }) = &app.editor
        else {
            panic!("expected a text editor");
        };
        assert_eq!(buf.len(), masterbus::MAX_EDITABLE_TEXT_BYTES);
    }

    /// A Text field whose value hasn't arrived yet has no sid to write to,
    /// so the edit is refused rather than guessing one.
    #[test]
    fn a_text_edit_needs_the_value_first() {
        let (mut app, _bus) = app();
        app.cur_device = Some(ADDR);
        app.rows = vec![Row::Field(field(
            field_id::btm1(0x01),
            "Device name",
            VisualizationType::Text,
            true,
        ))];
        app.row_sel = 0;

        app.begin_edit();
        assert!(!app.editing());
        assert_eq!(app.status, "Device name: value not loaded yet");
    }

    /// Keys aimed at an editor that isn't open go nowhere.
    #[test]
    fn editor_keys_without_an_editor_are_ignored() {
        let (mut app, _bus) = app();
        app.editor_char('1');
        app.editor_backspace();
        app.editor_choice_move(1);
        assert!(!app.editing());
    }

    // ── the values modal ────────────────────────────────────────────────────

    /// `?` lists every option of a list field and marks the current one.
    #[test]
    fn the_values_modal_lists_the_options_and_the_current_one() {
        let (mut app, _bus) = app();
        app.cur_device = Some(ADDR);
        let mut f = field(
            field_id::btm1(0x05),
            "Mode",
            VisualizationType::DropDown,
            true,
        );
        f.options = vec!["Off".into(), "On".into(), "Auto".into()];
        app.rows = vec![Row::Field(f)];
        app.row_sel = 0;
        app.values.insert(
            field_id::btm1(0x05),
            Value::List {
                index: 2,
                options: vec![],
            },
        );

        app.open_values();
        assert!(app.values_open());
        let modal = app.values_modal.as_ref().unwrap();
        assert_eq!(modal.field_name, "Mode");
        assert_eq!(modal.options, vec!["Off", "On", "Auto"]);
        assert_eq!(modal.current, Some(2));

        app.close_values();
        assert!(!app.values_open());
    }

    /// A field with no options says so rather than opening an empty modal.
    #[test]
    fn the_values_modal_refuses_a_field_without_options() {
        let (mut app, _bus) = app();
        seed_rows(&mut app);

        app.open_values();
        assert!(!app.values_open());
        assert_eq!(app.status, "Voltage: no list values to show");
    }

    // ── misc ────────────────────────────────────────────────────────────────

    /// The log pane only toggles when logging actually lands in the TUI.
    #[test]
    fn the_log_pane_only_toggles_when_logs_go_to_the_tui() {
        let (mut app, _bus) = app();
        assert!(!app.logs_in_tui);
        app.toggle_logs();
        assert!(!app.show_logs, "no pane to show without the tui-logger");

        app.logs_in_tui = true;
        app.toggle_logs();
        assert!(app.show_logs);
        app.toggle_logs();
        assert!(!app.show_logs);
    }

    #[test]
    fn quitting_sets_the_flag() {
        let (mut app, _bus) = app();
        assert!(!app.should_quit);
        app.quit();
        assert!(app.should_quit);
    }
}
