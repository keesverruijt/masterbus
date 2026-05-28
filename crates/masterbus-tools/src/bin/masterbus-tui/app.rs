//! TUI application state and the logic that mutates it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, TryRecvError};
use masterbus::{
    field_id, AccessLevel, DeviceIdentity, DeviceStatus, FieldId, FieldInfo, GroupInfo, MasterBus,
    Menu, Subscription, Value, VisualizationType,
};

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

    /// Drill into the selected device, landing on the Summary tab. The user
    /// can Tab into Monitoring / All Fields.
    pub fn open_device(&mut self) {
        let Some(&id) = self.device_ids.get(self.dev_sel) else { return };
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
        self.cur_access_level = self.bus.device(id).access_level().ok();
        self.status = format!("{} / Summary — Tab switch · Esc back", self.device_label(id));
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
        self.pending = Some(Pending { id, tab: TabKind::Menu(menu), name, started: Instant::now(), rx });
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
        self.pending = Some(Pending { id, tab: TabKind::Settings, name, started: Instant::now(), rx });
    }

    /// Whether a tab discovery is in flight.
    pub fn discovering(&self) -> bool {
        self.pending.is_some()
    }

    /// (name, tab, elapsed seconds) of the in-flight discovery, if any.
    pub fn pending_info(&self) -> Option<(&str, TabKind, u64)> {
        self.pending.as_ref().map(|p| (p.name.as_str(), p.tab, p.started.elapsed().as_secs()))
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
                        let fields = groups.into_iter().next().map(|g| g.fields).unwrap_or_default();
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
        let fields: Vec<FieldId> = groups.iter().flat_map(|g| g.fields.iter().map(|f| f.index)).collect();
        self.sub = if !fields.is_empty() {
            Some(self.bus.subscribe(id, fields, POLL_INTERVAL, false))
        } else {
            None
        };
        self.row_sel = 0;
        self.select_first_field();
        self.status =
            format!("{} / {} — Tab switch · Enter edit · Esc back", self.device_label(id), menu_label(menu));
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
            "{} / Settings ({} Btm3 fields) — Tab switch · Enter edit · Esc back",
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
        let Some(idx) = self.selected_field().map(|f| f.index) else { return };
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
            EditKind::Text { str_id, buf } => {
                self.write(ed.field, Value::Text { sid: str_id, text: buf });
            }
        }
    }

    pub fn editor_char(&mut self, c: char) {
        match &mut self.editor {
            Some(Editor { kind: EditKind::Number(buf), .. })
                if c.is_ascii_digit() || c == '.' || c == '-' =>
            {
                buf.push(c);
            }
            // Text fields are constrained on the wire: printable ASCII only,
            // ≤16 bytes. Enforce here so the user can't even type past it.
            Some(Editor { kind: EditKind::Text { buf, .. }, .. })
                if (c.is_ascii_graphic() || c == ' ')
                    && buf.len() < masterbus::MAX_EDITABLE_TEXT_BYTES =>
            {
                buf.push(c);
            }
            _ => {}
        }
    }

    pub fn editor_backspace(&mut self) {
        match &mut self.editor {
            Some(Editor { kind: EditKind::Number(buf), .. }) => {
                buf.pop();
            }
            Some(Editor { kind: EditKind::Text { buf, .. }, .. }) => {
                buf.pop();
            }
            _ => {}
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
            Some(LoginPrompt { stage: LoginStage::EnterPassword { .. }, .. })
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
        self.login = Some(LoginPrompt { device, sel, current, stage: LoginStage::PickLevel });
        self.status = "select access level — ↑/↓ pick · Enter next · Esc cancel".into();
    }

    pub fn login_move(&mut self, delta: i32) {
        if let Some(LoginPrompt { stage: LoginStage::PickLevel, sel, .. }) = &mut self.login {
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
        if let Some(LoginPrompt { stage: LoginStage::EnterPassword { buf, .. }, .. }) =
            &mut self.login
            && c.is_ascii_graphic()
        {
            buf.push(c);
        }
    }

    /// Pop the last char of the password buffer.
    pub fn login_backspace(&mut self) {
        if let Some(LoginPrompt { stage: LoginStage::EnterPassword { buf, .. }, .. }) =
            &mut self.login
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
        let Some(mut p) = self.login.take() else { return };
        match &p.stage {
            LoginStage::PickLevel => {
                let level = LOGIN_LEVELS[p.sel];
                if level == AccessLevel::EndUser {
                    let prev = p.current;
                    self.apply_login_result(p.device, level, self.bus.device(p.device).logout(), prev);
                } else {
                    // Stay in the modal; collect a password.
                    p.stage = LoginStage::EnterPassword { level, buf: String::new() };
                    self.status =
                        "enter password — type · Enter submit · Esc cancel".into();
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
