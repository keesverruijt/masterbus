//! TUI application state and the logic that mutates it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, TryRecvError};
use masterbus::{
    AccessLevel, DeviceIdentity, DeviceStatus, FieldInfo, GroupInfo, MasterBus, Menu, Subscription,
    Value, VisualizationType,
};

/// Live-poll rate for the selected device's monitoring fields.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// The tabs (menus) the UI exposes, in display order.
pub const TABS: [Menu; 3] = [Menu::Monitoring, Menu::Configuration, Menu::Service];

/// Device id → name, shared with the background name-backfill thread.
pub type Names = Arc<Mutex<HashMap<u32, String>>>;

/// A line in the field pane: either a group header or a field.
pub enum Row {
    Group(String),
    Field(FieldInfo),
}

/// Which pane has the keyboard focus.
#[derive(PartialEq, Eq)]
pub enum Focus {
    Devices,
    Fields,
}

/// An in-progress edit of the selected field.
pub struct Editor {
    pub field: i32,
    pub name: String,
    pub kind: EditKind,
}

pub enum EditKind {
    /// Free-text numeric entry.
    Number(String),
    /// Pick one of a fixed set of options.
    Choice { options: Vec<String>, sel: usize },
}

/// The fixed four access levels, in display order. Used by the login modal.
pub const LOGIN_LEVELS: [AccessLevel; 4] = [
    AccessLevel::EndUser,
    AccessLevel::Installer,
    AccessLevel::Distributor,
    AccessLevel::MvService,
];

/// A modal that picks an access level for the currently-selected device.
pub struct LoginPrompt {
    /// Target device id.
    pub device: u32,
    /// Highlighted option in [`LOGIN_LEVELS`].
    pub sel: usize,
    /// Level the device reports when the prompt opened (for display).
    pub current: Option<AccessLevel>,
}

/// A single-menu discovery running on a worker thread.
pub struct Pending {
    pub id: u32,
    pub menu: Menu,
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
    /// Whether the Information tab (device identity) is showing instead of a menu.
    pub on_info: bool,
    /// Cached identity for the Summary tab.
    pub cur_info: Option<DeviceIdentity>,
    /// Cached access level for the open device (shown in the title; refreshed
    /// on device open, after login, and on Summary-tab visit).
    pub cur_access_level: Option<AccessLevel>,
    /// The currently-displayed tab (menu).
    pub cur_menu: Menu,
    /// Menus already discovered for `cur_device`.
    pub loaded_menus: HashSet<Menu>,
    pub rows: Vec<Row>,
    pub row_sel: usize,
    pub values: HashMap<i32, Value>,
    pub sub: Option<Subscription>,
    pub editor: Option<Editor>,
    pub login: Option<LoginPrompt>,
    pub pending: Option<Pending>,
    pub tick: usize,
    pub status: String,
    pub should_quit: bool,
}

impl App {
    /// Construct without blocking: seed from whatever devices have been heard so
    /// far; the rest arrive via `note_alive`, and names via the backfill thread.
    pub fn new(bus: MasterBus, names: Names) -> App {
        let device_ids: Vec<u32> = bus.devices().iter().map(|d| d.id()).collect();
        App {
            bus,
            device_ids,
            names,
            dev_sel: 0,
            focus: Focus::Devices,
            cur_device: None,
            on_info: false,
            cur_info: None,
            cur_access_level: None,
            cur_menu: Menu::Monitoring,
            loaded_menus: HashSet::new(),
            rows: Vec::new(),
            row_sel: 0,
            values: HashMap::new(),
            sub: None,
            editor: None,
            login: None,
            pending: None,
            tick: 0,
            status: "scanning bus… ↑/↓ select · Enter open · l login · q quit".into(),
            should_quit: false,
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
        self.names.lock().unwrap().get(&id).cloned().unwrap_or_else(|| id.to_string())
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

    /// Drill into the selected device, showing the Monitoring tab first. Other
    /// tabs are discovered lazily when selected.
    pub fn open_device(&mut self) {
        let Some(&id) = self.device_ids.get(self.dev_sel) else { return };
        self.cur_device = Some(id);
        self.cur_menu = Menu::Monitoring;
        self.loaded_menus.clear();
        self.sub = None;
        self.rows.clear();
        self.values.clear();
        self.row_sel = 0;
        self.focus = Focus::Fields;
        // Open on the Summary tab; user can Tab into Monitoring/Config/Service.
        self.enter_info();
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
        // Tab 0 is Information; tabs 1.. are the menus in `TABS` order.
        let n = TABS.len() as i32 + 1;
        let cur = if self.on_info {
            0
        } else {
            1 + TABS.iter().position(|&m| m == self.cur_menu).unwrap_or(0) as i32
        };
        let next = ((cur + delta) % n + n) % n;
        if next == 0 {
            self.enter_info();
        } else {
            self.on_info = false;
            self.switch_tab(TABS[(next - 1) as usize]);
        }
    }

    /// Switch to the Summary tab: device identity + access level, no field list.
    fn enter_info(&mut self) {
        let Some(id) = self.cur_device else { return };
        self.on_info = true;
        self.sub = None;
        self.rows.clear();
        self.row_sel = 0;
        self.cur_info = self.bus.device(id).identity().ok();
        self.cur_access_level = self.bus.device(id).access_level().ok();
        self.status = format!("{} / Summary — Tab switch · Esc back", self.device_label(id));
    }

    fn switch_tab(&mut self, menu: Menu) {
        let Some(id) = self.cur_device else { return };
        self.cur_menu = menu;
        if self.loaded_menus.contains(&menu) {
            // Already discovered — rebuild instantly from the cached schema.
            let groups = self.bus.device(id).tab_info(menu).unwrap_or_default();
            self.show_tab(id, menu, groups);
        } else {
            self.rows.clear();
            self.row_sel = 0;
            self.start_tab_discovery(id, menu);
        }
    }

    /// Spawn a worker to discover one menu's groups (UI shows a spinner).
    fn start_tab_discovery(&mut self, id: u32, menu: Menu) {
        let name = self.device_label(id);
        self.status = format!("discovering {} / {}…", name, menu_label(menu));
        let (tx, rx) = bounded(1);
        let bus = self.bus.clone();
        std::thread::spawn(move || {
            if let Ok(groups) = bus.device(id).tab_info(menu) {
                let _ = tx.send(groups);
            }
        });
        self.pending = Some(Pending { id, menu, name, started: Instant::now(), rx });
    }

    /// Whether a tab discovery is in flight.
    pub fn discovering(&self) -> bool {
        self.pending.is_some()
    }

    /// (name, menu, elapsed seconds) of the in-flight discovery, if any.
    pub fn pending_info(&self) -> Option<(&str, Menu, u64)> {
        self.pending.as_ref().map(|p| (p.name.as_str(), p.menu, p.started.elapsed().as_secs()))
    }

    /// Check the discovery worker; when the menu's groups arrive, show them.
    pub fn poll_pending(&mut self) {
        let Some(p) = &self.pending else { return };
        match p.rx.try_recv() {
            Ok(groups) => {
                let (id, menu) = (p.id, p.menu);
                self.pending = None;
                self.loaded_menus.insert(menu);
                if let Ok(n) = self.bus.device(id).name() {
                    self.names.lock().unwrap().insert(id, n);
                }
                self.show_tab(id, menu, groups);
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

    fn show_tab(&mut self, id: u32, menu: Menu, groups: Vec<GroupInfo>) {
        self.build_rows(&groups);
        self.values.clear();
        // Monitoring fields get live updates; other tabs are read lazily on
        // selection (they're mostly static settings).
        let fields: Vec<i32> = groups.iter().flat_map(|g| g.fields.iter().map(|f| f.index)).collect();
        self.sub = if menu == Menu::Monitoring && !fields.is_empty() {
            Some(self.bus.subscribe(id, fields, POLL_INTERVAL, false))
        } else {
            None
        };
        self.row_sel = 0;
        self.select_first_field();
        self.status =
            format!("{} / {} — Tab switch · Enter edit · Esc back", self.device_label(id), menu_label(menu));
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
        self.on_info = false;
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
        let Some(idx) = self.selected_field().map(|f| f.index) else { return };
        self.ensure_value(idx);
    }

    fn ensure_value(&mut self, index: i32) {
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
        let Some(info) = self.selected_field().cloned() else { return };
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
                self.editor =
                    Some(Editor { field: info.index, name: info.name.clone(), kind: EditKind::Number(cur) });
            }
            V::Radio | V::DropDown => {
                let options = info.options.clone();
                let sel = match self.values.get(&info.index) {
                    Some(Value::List { index, .. }) => *index as usize,
                    _ => 0,
                };
                let sel = sel.min(options.len().saturating_sub(1));
                self.editor = Some(Editor {
                    field: info.index,
                    name: info.name.clone(),
                    kind: EditKind::Choice { options, sel },
                });
            }
            _ => self.status = format!("{}: not editable in this demo", info.name),
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editor = None;
        self.status = "edit cancelled".into();
    }

    pub fn commit_edit(&mut self) {
        let Some(ed) = self.editor.take() else { return };
        match ed.kind {
            EditKind::Number(buf) => match buf.trim().parse::<f32>() {
                Ok(f) => self.write(ed.field, Value::Float(f)),
                Err(_) => self.status = format!("'{buf}' is not a number"),
            },
            EditKind::Choice { options, sel } => {
                self.write(ed.field, Value::List { index: sel as i32, options });
            }
        }
    }

    pub fn editor_char(&mut self, c: char) {
        if let Some(Editor { kind: EditKind::Number(buf), .. }) = &mut self.editor
            && (c.is_ascii_digit() || c == '.' || c == '-')
        {
            buf.push(c);
        }
    }

    pub fn editor_backspace(&mut self) {
        if let Some(Editor { kind: EditKind::Number(buf), .. }) = &mut self.editor {
            buf.pop();
        }
    }

    pub fn editor_choice_move(&mut self, delta: i32) {
        if let Some(Editor { kind: EditKind::Choice { options, sel }, .. }) = &mut self.editor {
            if options.is_empty() {
                return;
            }
            let n = options.len() as i32;
            *sel = (((*sel as i32 + delta) % n + n) % n) as usize;
        }
    }

    fn write(&mut self, index: i32, value: Value) {
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
        self.login = Some(LoginPrompt { device, sel, current });
        self.status = "select access level — ↑/↓ pick · Enter login · Esc cancel".into();
    }

    pub fn login_move(&mut self, delta: i32) {
        if let Some(p) = &mut self.login {
            let n = LOGIN_LEVELS.len() as i32;
            p.sel = (((p.sel as i32 + delta) % n + n) % n) as usize;
        }
    }

    pub fn cancel_login(&mut self) {
        self.login = None;
        self.status = "login cancelled".into();
    }

    /// Apply the highlighted access level on the modal's target device.
    /// On success the engine drops its cached schema (writability re-queries),
    /// and we re-discover the currently-shown tab so the field list refreshes.
    pub fn commit_login(&mut self) {
        let Some(p) = self.login.take() else { return };
        let level = LOGIN_LEVELS[p.sel];
        let label = level_label(level);
        match self.bus.device(p.device).login(level) {
            Ok(reported) => {
                self.status =
                    format!("device 0x{:06X} → {} (reported: {})", p.device, label, level_label(reported));
                // If we're currently inside this device, reload everything the
                // UI was showing (schema attributes, identity, title).
                if self.cur_device == Some(p.device) {
                    self.cur_access_level = Some(reported);
                    self.loaded_menus.clear();
                    self.values.clear();
                    self.sub = None;
                    if self.on_info {
                        // Refresh the identity card; nothing to discover here.
                        self.cur_info = self.bus.device(p.device).identity().ok();
                    } else {
                        let menu = self.cur_menu;
                        self.rows.clear();
                        self.row_sel = 0;
                        self.start_tab_discovery(p.device, menu);
                    }
                }
            }
            Err(e) => {
                self.status = format!("login({label}) on 0x{:06X} failed: {e}", p.device);
            }
        }
    }
}

pub fn menu_label(menu: Menu) -> String {
    match menu {
        Menu::Monitoring => "Monitoring".into(),
        Menu::Configuration => "Config".into(),
        Menu::Service => "Service".into(),
        Menu::Other(s) => format!("Menu {s:#04x}"),
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
