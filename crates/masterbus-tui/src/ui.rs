//! Rendering of the [`App`] state with ratatui.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::Frame;

use masterbus::{DeviceStatus, Value};

use crate::app::{menu_label, App, EditKind, Focus, Row, TABS};

/// Braille spinner frames.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(f.area());
    let panes = Layout::horizontal([Constraint::Length(34), Constraint::Min(0)]).split(outer[0]);
    draw_devices(f, app, panes[0]);
    draw_fields(f, app, panes[1]);
    draw_footer(f, app, outer[1]);
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

    let block = bordered(app.device_label(id), app.focus == Focus::Fields);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Tab bar (menus) + content below it.
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let titles: Vec<Line> = TABS.iter().map(|&m| Line::raw(menu_label(m))).collect();
    let sel = TABS.iter().position(|&m| m == app.cur_menu).unwrap_or(0);
    f.render_widget(
        Tabs::new(titles)
            .select(sel)
            .highlight_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::REVERSED)),
        parts[0],
    );
    let content = parts[1];

    // While a tab is being discovered, show an animated progress panel.
    if let Some((name, menu, secs)) = app.pending_info() {
        let spin = SPINNER[app.tick % SPINNER.len()];
        let lines = vec![
            Line::raw(""),
            Line::from(Span::styled(
                format!("{spin}  Discovering {name} / {}…  ({secs}s)", menu_label(menu)),
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
                let val = app.values.get(&field.index).map(format_value).unwrap_or_else(|| "…".into());
                let rw = if field.writeable { "rw" } else { "ro" };
                ListItem::new(Line::raw(format!(
                    "  {:<22} {:>10} {:<4} {}",
                    truncate(&field.name, 22),
                    val,
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

pub fn format_value(v: &Value) -> String {
    match v {
        Value::Float(x) if x.is_nan() => "—".into(),
        Value::Float(x) => format!("{x:.2}"),
        Value::Boolean(b) => if *b { "on" } else { "off" }.into(),
        Value::Date(d) => format!("{:04}-{:02}-{:02}", d.year, d.mon, d.day),
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
