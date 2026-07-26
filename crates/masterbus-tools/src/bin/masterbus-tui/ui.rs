//! Rendering of the [`App`] state with ratatui.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};

use masterbus::{DeviceStatus, FieldId, Value, VisualizationType};

use crate::app::{App, EditKind, Focus, LOGIN_LEVELS, Row, TABS, TabKind, level_label, tab_label};

/// Braille spinner frames.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Width of the value column (fits "Nd HH:MM:SS" time values without overflow).
const VALUE_COL: usize = 21;

pub fn draw(f: &mut Frame, app: &App) {
    let outer = if app.show_logs {
        // Main pane shrinks; 8-line log pane above the footer.
        Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(f.area())
    } else {
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(f.area())
    };
    let panes = Layout::horizontal([Constraint::Length(34), Constraint::Min(0)]).split(outer[0]);
    draw_devices(f, app, panes[0]);
    draw_fields(f, app, panes[1]);
    if app.show_logs {
        draw_logs(f, outer[1]);
        draw_footer(f, app, outer[2]);
    } else {
        draw_footer(f, app, outer[1]);
    }
    if app.login.is_some() {
        draw_login(f, app, f.area());
    }
    if app.editor.is_some() {
        draw_edit_modal(f, app, f.area());
    }
    if app.values_modal.is_some() {
        draw_values_modal(f, app, f.area());
    }
}

fn draw_logs(f: &mut Frame, area: Rect) {
    use tui_logger::{TuiLoggerLevelOutput, TuiLoggerWidget};
    let widget = TuiLoggerWidget::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Logs (~ to hide) "),
        )
        .style_error(Style::default().fg(Color::Red))
        .style_warn(Style::default().fg(Color::Yellow))
        .style_info(Style::default().fg(Color::Green))
        .style_debug(Style::default().fg(Color::Cyan))
        .style_trace(Style::default().fg(Color::DarkGray))
        .output_separator(' ')
        .output_timestamp(None)
        .output_level(Some(TuiLoggerLevelOutput::Abbreviated))
        .output_target(true)
        .output_file(false)
        .output_line(false);
    f.render_widget(widget, area);
}

fn draw_devices(f: &mut Frame, app: &App, area: Rect) {
    use masterbus::AccessLevel;
    let items: Vec<ListItem> = app
        .device_ids
        .iter()
        .map(|&id| {
            let (sym, color) = status_style(app.device_status(id));
            // Append " (<level>)" when logged in past End User — uses the
            // cached level only, no wire query (cheap to call per render).
            let mut label = app.device_label(id);
            if let Some(lvl) = app.bus.device(id).cached_access_level()
                && lvl != AccessLevel::EndUser
            {
                label.push_str(&format!(" ({})", crate::app::level_label(lvl)));
            }
            Line::from(vec![
                Span::styled(format!("{sym} "), Style::new().fg(color)),
                Span::raw(label),
            ])
            .into()
        })
        .collect();

    let mut state = ListState::default();
    if !app.device_ids.is_empty() {
        state.select(Some(app.dev_sel));
    }

    let list = List::new(items)
        .block(bordered(
            format!("Devices ({})", app.device_ids.len()),
            app.focus == Focus::Devices,
        ))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_fields(f: &mut Frame, app: &App, area: Rect) {
    let Some(id) = app.cur_device else {
        f.render_widget(bordered("Fields".into(), app.focus == Focus::Fields), area);
        return;
    };

    let level_suffix = app
        .cur_access_level
        .map(|l| format!("  · {}", level_label(l)))
        .unwrap_or_default();
    let block = bordered(
        format!("{}  [{:06X}]{}", app.device_label(id), id, level_suffix),
        app.focus == Focus::Fields,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Tab bar + content below it.
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let titles: Vec<Line> = TABS.iter().map(|&t| Line::raw(tab_label(t))).collect();
    let sel = TABS.iter().position(|&t| t == app.cur_tab).unwrap_or(0);
    f.render_widget(
        Tabs::new(titles).select(sel).highlight_style(
            Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::REVERSED),
        ),
        parts[0],
    );
    let content = parts[1];

    // Summary tab: device identity, not a field list.
    if app.cur_tab == TabKind::Summary {
        draw_info(f, app, id, content);
        return;
    }

    // While a tab is being discovered, show an animated progress panel.
    if let Some((name, tab, secs)) = app.pending_info() {
        let spin = SPINNER[app.tick % SPINNER.len()];
        let lines = vec![
            Line::raw(""),
            Line::from(Span::styled(
                format!(
                    "{spin}  Discovering {name} / {}…  ({secs}s)",
                    tab_label(tab)
                ),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "enumerating this tab's groups & fields",
                Style::new().fg(Color::DarkGray),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "Esc to cancel",
                Style::new().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), content);
        return;
    }

    // Snapshot device-name map once for resolving event-target device refs.
    let names = app.names.lock().unwrap();
    // An `Event N command` selects an output of the event's *target* device,
    // which is the immediately-preceding `Event N target` field. Track the last
    // target device id seen so a command row can resolve its output's name.
    let mut last_target: Option<u32> = None;
    let mut items: Vec<ListItem> = Vec::with_capacity(app.rows.len());
    for row in &app.rows {
        let item = match row {
            Row::Group(name) => ListItem::new(Span::styled(
                name.clone(),
                Style::new().add_modifier(Modifier::BOLD).fg(Color::Yellow),
            )),
            Row::Field(field) => {
                let value = app.values.get(&field.index);
                // Remember the target device a following command row acts on.
                if field.viz_type == VisualizationType::DeviceList {
                    last_target = match value {
                        Some(Value::DeviceRef { index, .. }) => {
                            app.device_ids.get(*index as usize).copied()
                        }
                        _ => None,
                    };
                }
                let val = match (field.viz_type, value) {
                    // Resolve the command index to the target's output name.
                    (VisualizationType::EventCommand, Some(v)) => {
                        command_label(v.index(), last_target, app)
                    }
                    (_, Some(v)) => format_value_for(v, &field.options, &app.device_ids, &names),
                    (_, None) => "…".into(),
                };
                // Cap to the column width so a long value (e.g. a "0d HH:MM:SS"
                // time) can't push the unit column out of alignment.
                let val = truncate(&val, VALUE_COL);
                let rw = if field.writeable { "rw" } else { "ro" };
                ListItem::new(Line::raw(format!(
                    "  {} {:<22} {rw} {val:>VALUE_COL$} {}",
                    field_id_tag(field.index),
                    truncate(&field.name, 22),
                    field.unit,
                )))
            }
        };
        items.push(item);
    }

    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.row_sel));
    }

    let list = List::new(items)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    f.render_stateful_widget(list, content, &mut state);
}

fn draw_info(f: &mut Frame, app: &App, id: u32, area: Rect) {
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("  {k:<11}"), Style::new().fg(Color::DarkGray)),
            Span::raw(v),
        ])
    };
    let mut lines = vec![Line::raw("")];
    if let Some(info) = &app.cur_info {
        lines.push(row("Name", info.name.clone()));
        lines.push(row("Device id", format!("{id:06X}")));
        lines.push(row("Article", info.article.clone()));
        lines.push(row("Serial", info.serial.clone()));
        lines.push(row("Revision", info.revision.clone()));
        lines.push(row("Firmware", info.firmware.clone()));
        let status = app.device_status(id);
        let (sym, color) = status_style(status);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<11}", "Status"),
                Style::new().fg(Color::DarkGray),
            ),
            Span::styled(format!("{sym} {status:?}"), Style::new().fg(color)),
        ]));
        let access = app
            .cur_access_level
            .map(level_label)
            .unwrap_or("—")
            .to_string();
        lines.push(row("Access", access));
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  press l to log in / out",
            Style::new().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::raw("  (identity unavailable)"));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_login(f: &mut Frame, app: &App, area: Rect) {
    let Some(prompt) = &app.login else { return };
    // 50×11 centred overlay.
    let w = 50u16.min(area.width.saturating_sub(2));
    let h = 11u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(ratatui::widgets::Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Cyan))
        .title(format!(" Login → device 0x{:06X} ", prompt.device));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::with_capacity(10);
    let current = prompt
        .current
        .map(|l| format!("currently: {}", level_label(l)))
        .unwrap_or_else(|| "currently: (unknown)".into());
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        format!("  {current}"),
        Style::new().fg(Color::DarkGray),
    )));
    lines.push(Line::raw(""));

    match &prompt.stage {
        crate::app::LoginStage::PickLevel => {
            for (i, &level) in LOGIN_LEVELS.iter().enumerate() {
                let marker = if i == prompt.sel { "› " } else { "  " };
                let style = if i == prompt.sel {
                    Style::new().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::new()
                };
                lines.push(Line::from(Span::styled(
                    format!("{marker}{}", level_label(level)),
                    style,
                )));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  ↑/↓ pick · Enter next · Esc cancel",
                Style::new().fg(Color::DarkGray),
            )));
        }
        crate::app::LoginStage::EnterPassword { level, buf } => {
            lines.push(Line::from(Span::styled(
                format!("  log in as {}", level_label(*level)),
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::raw(""));
            // Mask the password chars with bullets.
            let masked: String = "•".repeat(buf.chars().count());
            lines.push(Line::from(Span::styled(
                format!("  password: {masked}_"),
                Style::new().fg(Color::White).bg(Color::DarkGray),
            )));
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  type · Enter submit · Backspace · Esc cancel",
                Style::new().fg(Color::DarkGray),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    // The status line; the in-progress edit moves to a centred modal
    // (see [`draw_edit_modal`]).
    let style = Style::new().fg(Color::Gray);
    f.render_widget(
        Paragraph::new(format!(" {}", app.status)).style(style),
        area,
    );
}

fn draw_edit_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(ed) = &app.editor else { return };
    // 60×9 centred (clamped to terminal size). Wide enough for a 22-char
    // field name + the 16-char text limit + a margin.
    let w = 60u16.min(area.width.saturating_sub(2));
    let h = 9u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(ratatui::widgets::Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Cyan))
        .title(format!(" Edit · {} ", ed.name));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let (body, hint) = match &ed.kind {
        EditKind::Number(buf) => (format!(" {buf}_ "), "Enter ok  ·  Esc cancel".to_string()),
        EditKind::Choice { options, sel } => (
            format!(
                " ‹ {} ›   ({}/{}) ",
                options.get(*sel).map(String::as_str).unwrap_or("?"),
                sel + 1,
                options.len()
            ),
            "←/→ change  ·  Enter ok  ·  Esc cancel".to_string(),
        ),
        EditKind::Text { str_id, buf } => (
            format!(" \"{buf}_\" "),
            format!(
                "sid 0x{:04X}  ·  {}/{} chars  ·  Enter ok  ·  Esc cancel",
                str_id,
                buf.len(),
                masterbus::MAX_EDITABLE_TEXT_BYTES
            ),
        ),
    };

    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            body,
            Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::styled(hint, Style::new().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}

fn draw_values_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(view) = &app.values_modal else {
        return;
    };

    // Width fits the widest "Label (index)" line plus a margin; height fits
    // every option plus title + border.
    let widest = view
        .options
        .iter()
        .enumerate()
        .map(|(i, s)| s.chars().count() + 1 + 1 + i.to_string().len() + 1)
        .max()
        .unwrap_or(20);
    let w = (widest as u16 + 4).clamp(28, area.width.saturating_sub(2));
    let h = (view.options.len() as u16 + 2).clamp(5, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(ratatui::widgets::Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Cyan))
        .title(format!(" Values · {} ", view.field_name));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines: Vec<Line> = view
        .options
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let line = format!("{label} ({i})");
            if Some(i as i32) == view.current {
                Line::from(Span::styled(
                    format!("» {line}"),
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::raw(format!("  {line}")))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn bordered(title: String, focused: bool) -> Block<'static> {
    let color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(color))
        .title(title)
}

fn status_style(status: DeviceStatus) -> (&'static str, Color) {
    use DeviceStatus as S;
    match status {
        S::On => ("●", Color::Green),
        S::OnWarning => ("●", Color::Yellow),
        S::Sleeping => ("◐", Color::Blue),
        S::OffFault | S::OffError => ("●", Color::Red),
        S::Updating => ("⟳", Color::Magenta),
        S::Offline => ("○", Color::DarkGray),
        S::Unknown => ("?", Color::DarkGray),
    }
}

/// Format a value, resolving a list/enum index to its label using the value's
/// own option strings if present, else the field's schema options. A
/// [`Value::DeviceRef`] (an event target) is resolved to the referenced
/// device's name via the address-sorted bus device list.
fn format_value_for(
    v: &Value,
    schema_opts: &[String],
    devices: &[u32],
    names: &HashMap<u32, String>,
) -> String {
    // Append the raw integer index for list/eventable values so the underlying
    // wire value is visible alongside the human label: `Stabilized(2)`.
    let label = |index: i32, value_opts: &[String]| -> String {
        let src = if value_opts.is_empty() {
            schema_opts
        } else {
            value_opts
        };
        let text = src
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| format!("[{index}]"));
        let text = truncate(&text, 16);
        format!("{text}({index})")
    };
    match v {
        Value::List { index, options } => label(*index, options),
        Value::Eventable { index, labels } => label(*index, labels),
        Value::DeviceRef { index, .. } => device_ref_label(*index, devices, names),
        _ => format_value(v),
    }
}

/// Resolve an event `command` index to the target device's output name.
/// `command = K` selects the target's `K`-th eventable output (see PROTOCOL
/// §9a). Falls back to `output K` when the target's config isn't discovered yet
/// (so its outputs aren't cached) or the index is out of range.
fn command_label(index: Option<i32>, target: Option<u32>, app: &App) -> String {
    let Some(k) = index else { return "…".into() };
    let name = target
        .map(|id| app.bus.device(id).eventable_outputs())
        .and_then(|outs| usize::try_from(k).ok().and_then(|i| outs.get(i).cloned()))
        .filter(|n| !n.is_empty());
    match name {
        Some(n) => truncate(&n, VALUE_COL),
        None => format!("output {k}"),
    }
}

/// Resolve an event-target device reference to a name. The stored value is a
/// 0-based index into the bus device list sorted by device address — the same
/// canonical order [`MasterBus::device_ids`] returns (see FINDINGS) — so
/// `devices[index]` is the target device id, and `names` maps it to a name.
/// Falls back to the hex id (name not yet backfilled) or `[index]` (index out
/// of range, e.g. a referenced device currently offline).
fn device_ref_label(index: i32, devices: &[u32], names: &HashMap<u32, String>) -> String {
    match usize::try_from(index).ok().and_then(|i| devices.get(i)) {
        Some(&id) => {
            let name = names
                .get(&id)
                .filter(|n| !n.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("0x{id:06X}"));
            format!("→ {}", truncate(&name, 18))
        }
        None => format!("→ [{index}]"),
    }
}

pub fn format_value(v: &Value) -> String {
    match v {
        Value::Float(x) if x.is_nan() => "—".into(),
        Value::Float(x) => format!("{x:.2}"),
        Value::Boolean(b) => if *b { "on" } else { "off" }.into(),
        // -1 in any component is the device's "no value" sentinel.
        Value::Date(d) if d.year < 0 || d.mon < 0 || d.day < 0 => "—".into(),
        Value::Date(d) => format!("{:04}-{:02}-{:02}", d.year, d.mon, d.day),
        Value::Time(t) if t.sec < 0 => "—".into(),
        Value::Time(t) => format!("{}d {:02}:{:02}:{:02}", t.days, t.hour, t.min, t.sec),
        Value::List { index, options } => options
            .get(*index as usize)
            .cloned()
            .unwrap_or_else(|| format!("[{index}]")),
        Value::Text { text, .. } => text.clone(),
        Value::DeviceRef { index, device_ids } => {
            format!(
                "->{}",
                device_ids.get(*index as usize).copied().unwrap_or(0)
            )
        }
        Value::Eventable { index, labels } => labels
            .get(*index as usize)
            .cloned()
            .unwrap_or_else(|| format!("[{index}]")),
        Value::Invalid => "invalid".into(),
    }
}

/// Render a channel-aware [`FieldId`] as `0x000`..`0x1FF` — three hex digits
/// of the full `u16` id, where bit 8 encodes the channel (`0x000`..`0x0FF` =
/// Btm1, `0x100`..`0x1FF` = Btm3). Five chars wide, matches the encoding the
/// `masterbus-set-field` CLI takes as its `<field_id>` argument.
fn field_id_tag(id: FieldId) -> String {
    format!("0x{id:03X}")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ref_resolves_index_to_device_name() {
        // Real 24-bit device_addr values (what device_ids() returns), sorted
        // ascending — CombiMaster, Alternator, Watt & Sea, Solar.
        let devices = [0x188EA2u32, 0x30472A, 0x386FFD, 0x387028];
        let names: HashMap<u32, String> = [
            (0x386FFD, "Watt & Sea".to_string()),
            (0x387028, "Solar".to_string()),
        ]
        .into_iter()
        .collect();

        // index 2 -> Watt & Sea, index 3 -> Solar (matches the reference bus).
        assert_eq!(device_ref_label(2, &devices, &names), "→ Watt & Sea");
        assert_eq!(device_ref_label(3, &devices, &names), "→ Solar");
        // Name not yet backfilled -> hex id fallback (same 06X form the UI uses).
        assert_eq!(device_ref_label(0, &devices, &names), "→ 0x188EA2");
        // Out of range (referenced device offline) -> raw index.
        assert_eq!(device_ref_label(9, &devices, &names), "→ [9]");
    }

    #[test]
    fn device_ref_value_renders_via_format_value_for() {
        let devices = [0x386FFDu32, 0x387028];
        let names: HashMap<u32, String> = [(0x387028, "Solar".to_string())].into_iter().collect();
        let v = Value::DeviceRef {
            index: 1,
            device_ids: Vec::new(),
        };
        assert_eq!(format_value_for(&v, &[], &devices, &names), "→ Solar");
    }
}
