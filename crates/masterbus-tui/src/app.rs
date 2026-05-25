//! TUI application state and the logic that mutates it.

use std::collections::HashMap;
use std::time::Duration;

use masterbus::{
    DeviceSchema, DeviceStatus, FieldInfo, MasterBus, Menu, Subscription, Value, VisualizationType,
};

/// Live-poll rate for the selected device's monitoring fields.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

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

pub struct App {
    pub bus: MasterBus,
    pub device_ids: Vec<u32>,
    pub names: HashMap<u32, String>,
    pub dev_sel: usize,
    pub focus: Focus,
    pub cur_device: Option<u32>,
    pub rows: Vec<Row>,
    pub row_sel: usize,
    pub values: HashMap<i32, Value>,
    pub sub: Option<Subscription>,
    pub editor: Option<Editor>,
    pub status: String,
    pub should_quit: bool,
}

impl App {
    pub fn new(bus: MasterBus) -> App {
        let device_ids: Vec<u32> = bus.devices_all().iter().map(|d| d.id()).collect();
        App {
            bus,
            device_ids,
            names: HashMap::new(),
            dev_sel: 0,
            focus: Focus::Devices,
            cur_device: None,
            rows: Vec::new(),
            row_sel: 0,
            values: HashMap::new(),
            sub: None,
            editor: None,
            status: "↑/↓ select · Enter open · q quit".into(),
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
        self.names.get(&id).cloned().unwrap_or_else(|| id.to_string())
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

    /// Drill into the selected device: discover, build rows, subscribe.
    pub fn open_device(&mut self) {
        let Some(&id) = self.device_ids.get(self.dev_sel) else { return };
        let dev = self.bus.device(id);
        let schema = match dev.schema() {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("discovery failed: {e}");
                return;
            }
        };
        self.names.insert(id, schema.name.clone());
        self.cur_device = Some(id);
        self.values.clear();
        self.build_rows(&schema);
        self.subscribe_monitoring(id, &schema);
        self.focus = Focus::Fields;
        self.row_sel = 0;
        self.select_first_field();
        self.status = format!("{} — Enter edit · Esc back · q quit", schema.name);
    }

    fn build_rows(&mut self, schema: &DeviceSchema) {
        self.rows.clear();
        for g in &schema.groups {
            self.rows.push(Row::Group(format!("[{}] {}", menu_label(g.menu), g.name)));
            for f in &g.fields {
                self.rows.push(Row::Field(f.clone()));
            }
        }
    }

    fn subscribe_monitoring(&mut self, id: u32, schema: &DeviceSchema) {
        let fields: Vec<i32> = schema
            .groups
            .iter()
            .filter(|g| g.menu == Menu::Monitoring)
            .flat_map(|g| g.fields.iter().map(|f| f.index))
            .collect();
        self.sub = if fields.is_empty() {
            None
        } else {
            Some(self.bus.subscribe(id, fields, POLL_INTERVAL, false))
        };
    }

    pub fn back_to_devices(&mut self) {
        self.focus = Focus::Devices;
        self.sub = None;
        self.rows.clear();
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
        if let Some(Editor { kind: EditKind::Number(buf), .. }) = &mut self.editor {
            if c.is_ascii_digit() || c == '.' || c == '-' {
                buf.push(c);
            }
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
}

fn menu_label(menu: Menu) -> String {
    match menu {
        Menu::Monitoring => "Monitoring".into(),
        Menu::Configuration => "Config".into(),
        Menu::Service => "Service".into(),
        Menu::Other(s) => format!("Menu {s:#04x}"),
    }
}
