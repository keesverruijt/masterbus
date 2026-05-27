//! Rendering of the [`App`] state with ratatui.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::Frame;

use masterbus::{DeviceStatus, Value};

use crate::app::{level_label, tab_label, App, EditKind, Focus, Row, TabKind, LOGIN_LEVELS, TABS};

/// Braille spinner frames.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Width of the value column (fits "Nd HH:MM:SS" time values without overflow).
const VALUE_COL: usize = 11;

pub fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(f.area());
    let panes = Layout::horizontal([Constraint::Length(34), Constraint::Min(0)]).split(outer[0]);
    draw_devices(f, app, panes[0]);
    draw_fields(f, app, panes[1]);
    draw_footer(f, app, outer[1]);
    if app.login.is_some() {
        draw_login(f, app, f.area());
    }
}

fn draw_devices(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .device_ids
        .iter()
        .map(|&id| {
            let (sym, color) = status_style(app.device_status(id));
            Line::from(vec![
                Span::styled(format!("{sym} "), Style::new().fg(color)),
                Span::raw(app.device_label(id)),
            ])
            .into()
        })
        .collect();

    let mut state = ListState::default();
    if !app.device_ids.is_empty() {
        state.select(Some(app.dev_sel));
    }

    let list = List::new(items)
        .block(bordered(format!("Devices ({})", app.device_ids.len()), app.focus == Focus::Devices))
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
        Tabs::new(titles)
            .select(sel)
            .highlight_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::REVERSED)),
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
                format!("{spin}  Discovering {name} / {}…  ({secs}s)", tab_label(tab)),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "enumerating this tab's groups & fields",
                Style::new().fg(Color::DarkGray),
            )),
            Line::raw(""),
            Line::from(Span::styled("Esc to cancel", Style::new().fg(Color::DarkGray))),
        ];
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), content);
        return;
    }

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| match row {
            Row::Group(name) => ListItem::new(Span::styled(
                name.clone(),
                Style::new().add_modifier(Modifier::BOLD).fg(Color::Yellow),
            )),
            Row::Field(field) => {
                let val = app
                    .values
                    .get(&field.index)
                    .map(|v| format_value_for(v, &field.options))
                    .unwrap_or_else(|| "…".into());
                // Cap to the column width so a long value (e.g. a "0d HH:MM:SS"
                // time) can't push the unit / rw column out of alignment.
                let val = truncate(&val, VALUE_COL);
                let rw = if field.writeable { "rw" } else { "ro" };
                ListItem::new(Line::raw(format!(
                    "  {:<22} {val:>VALUE_COL$} {:<4} {}",
                    truncate(&field.name, 22),
                    field.unit,
                    rw
                )))
            }
        })
        .collect();

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
            Span::styled(format!("  {:<11}", "Status"), Style::new().fg(Color::DarkGray)),
            Span::styled(format!("{sym} {status:?}"), Style::new().fg(color)),
        ]));
        let access = app.cur_access_level.map(level_label).unwrap_or("—").to_string();
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

    let mut lines: Vec<Line> = Vec::with_capacity(8);
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
    for (i, &level) in LOGIN_LEVELS.iter().enumerate() {
        let marker = if i == prompt.sel { "› " } else { "  " };
        let style = if i == prompt.sel {
            Style::new().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::new()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{}", level_label(level)), style)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ pick · Enter login · Esc cancel",
        Style::new().fg(Color::DarkGray),
    )));

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let (text, style) = match &app.editor {
        Some(ed) => {
            let body = match &ed.kind {
                EditKind::Number(buf) => {
                    format!(" edit {} = {}_   (Enter ok · Esc cancel)", ed.name, buf)
                }
                EditKind::Choice { options, sel } => format!(
                    " edit {} ‹ {} ›   (←/→ change · Enter ok · Esc cancel)",
                    ed.name,
                    options.get(*sel).map(String::as_str).unwrap_or("?")
                ),
            };
            (body, Style::new().fg(Color::Black).bg(Color::Cyan))
        }
        None => (format!(" {}", app.status), Style::new().fg(Color::Gray)),
    };
    f.render_widget(Paragraph::new(text).style(style), area);
}

fn bordered(title: String, focused: bool) -> Block<'static> {
    let color = if focused { Color::Cyan } else { Color::DarkGray };
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
/// own option strings if present, else the field's schema options.
fn format_value_for(v: &Value, schema_opts: &[String]) -> String {
    let label = |index: i32, value_opts: &[String]| -> String {
        let src = if value_opts.is_empty() { schema_opts } else { value_opts };
        src.get(index as usize).cloned().unwrap_or_else(|| format!("[{index}]"))
    };
    match v {
        Value::List { index, options } => label(*index, options),
        Value::Eventable { index, labels } => label(*index, labels),
        _ => format_value(v),
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
        Value::List { index, options } => {
            options.get(*index as usize).cloned().unwrap_or_else(|| format!("[{index}]"))
        }
        Value::Text(s) => s.clone(),
        Value::DeviceRef { index, device_ids } => {
            format!("->{}", device_ids.get(*index as usize).copied().unwrap_or(0))
        }
        Value::Eventable { index, labels } => {
            labels.get(*index as usize).cloned().unwrap_or_else(|| format!("[{index}]"))
        }
        Value::Invalid => "invalid".into(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
