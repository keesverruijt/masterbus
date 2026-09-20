//! Rendering of the [`App`] state with ratatui.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap};

use masterbus::{DeviceStatus, FieldId, Value, VisualizationType};
use masterbus_tools::mapping::NotifyState;

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
    if app.path_editor.is_some() {
        draw_path_modal(f, app, f.area());
    }
}

/// The mapping editor's path prompt: the field being mapped, the path being
/// typed, and the conversion that path implies.
///
/// Showing the conversion is the point. The mapping file stores no scale
/// factor, so the only moment a human can check that °C is about to become
/// kelvin is while they are choosing the path.
fn draw_path_modal(f: &mut Frame, app: &App, area: Rect) {
    use crate::app::{Hint, Origin, Stage};
    let Some(ed) = app.path_editor.as_ref() else {
        return;
    };
    if let Stage::Truth(sel) = ed.stage {
        return draw_truth_modal(f, ed, sel, area);
    }
    if let Stage::Notify(sel) = ed.stage {
        return draw_notify_modal(f, ed, sel, area);
    }
    let w = area.width.saturating_sub(4).clamp(30, 84);

    let origin = match ed.origin {
        Origin::Existing => "editing the existing mapping".to_string(),
        Origin::Suggested(t) => t.describe().to_string(),
        Origin::Blank => "no suggestion for this field".to_string(),
    };
    let (hint, hint_style) = match ed.hint() {
        Hint::Ok(h) => (h, Style::new().fg(Color::Green)),
        Hint::Warn(h) => (h, Style::new().fg(Color::Yellow)),
        Hint::Refuse(h) => (h, Style::new().fg(Color::Red)),
    };
    let unit = if ed.unit.trim().is_empty() {
        "no unit".to_string()
    } else {
        format!("in {}", ed.unit)
    };
    // For an enum on a boolean leaf ^N flips the table shown in the hint;
    // a separate "inverted" flag would be one more thing to apply mentally.
    let mut invert_line = if app.flips_truth() {
        "^N flips true/false".to_string()
    } else {
        format!(
            "invert: {}  (^N toggles)",
            if ed.invert { "yes" } else { "no" }
        )
    };
    if !ed.options.is_empty() {
        invert_line.push_str(" · ^A notifications");
    }
    let body = vec![
        Line::from(Span::styled(
            format!("{} ({unit})", ed.field_name),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(origin, Style::new().fg(Color::DarkGray))),
        Line::raw(""),
        Line::from(Span::styled(
            format!("{}\u{2588}", ed.buf),
            Style::new().fg(Color::Cyan),
        )),
        Line::from(Span::styled(hint, hint_style)),
        Line::from(Span::styled(invert_line, Style::new().fg(Color::DarkGray))),
    ];
    draw_modal(
        f,
        area,
        w,
        &body,
        format!(" Signal K path for {} ", field_id_tag(ed.field)),
        " Enter save · Esc cancel ",
    );
}

/// A centred, bordered box sized to its wrapped content. Signal K paths and
/// the hints about them run long, and an 80-column terminal is normal on a
/// boat, so everything wraps rather than being cut off at the border.
fn draw_modal(f: &mut Frame, area: Rect, w: u16, body: &[Line<'_>], title: String, foot: &str) {
    let inner = w.saturating_sub(4) as usize; // borders + one column padding
    let rows: u16 = body.iter().map(|l| wrapped_rows(l.width(), inner)).sum();
    let h = (rows + 2).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);
    f.render_widget(ratatui::widgets::Clear, rect);
    let p = Paragraph::new(body.to_vec())
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(ratatui::widgets::Padding::horizontal(1))
                .title(title)
                .title_bottom(foot),
        );
    f.render_widget(p, rect);
}

/// Rows a line of `width` cells occupies when wrapped into `cols` columns.
fn wrapped_rows(width: usize, cols: usize) -> u16 {
    if cols == 0 {
        return 1;
    }
    width.max(1).div_ceil(cols) as u16
}

/// The truth-table stage of the path prompt: an enum is going to a boolean
/// leaf and this build could not classify every label, so the user says which
/// labels mean `true`. Conventional labels arrive pre-filled; the rest are
/// blank until chosen.
fn draw_truth_modal(f: &mut Frame, ed: &crate::app::PathEditor, sel: usize, area: Rect) {
    let w = area.width.saturating_sub(4).clamp(30, 84);
    let mut body = vec![
        Line::from(Span::styled(
            format!("{} → {}", ed.field_name, ed.buf.trim()),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "a boolean leaf: which labels mean true?",
            Style::new().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ];
    for (i, label) in ed.options.iter().enumerate() {
        let marker = if i == sel { "› " } else { "  " };
        let (value, style) = match ed.truth.get(label) {
            Some(true) => ("true", Style::new().fg(Color::Green)),
            Some(false) => ("false", Style::new().fg(Color::Red)),
            None => ("?", Style::new().fg(Color::Yellow)),
        };
        let label_style = if i == sel {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        body.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(format!("{label:<20}"), label_style),
            Span::styled(value, style),
        ]));
    }
    draw_modal(
        f,
        area,
        w,
        &body,
        format!(" Truth table for {} ", field_id_tag(ed.field)),
        " Space/t/f set · ^N flip all · Enter save · Esc back ",
    );
}

/// The notification stage of the path prompt: which of an enum's labels
/// should raise a Signal K notification, and how loudly. Labels that sound
/// like trouble arrive pre-filled.
fn draw_notify_modal(f: &mut Frame, ed: &crate::app::PathEditor, sel: usize, area: Rect) {
    let w = area.width.saturating_sub(4).clamp(30, 84);
    let mut body = vec![
        Line::from(Span::styled(
            format!("{} → {}", ed.field_name, ed.buf.trim()),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "which labels should raise a notification?",
            Style::new().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ];
    for (i, label) in ed.options.iter().enumerate() {
        let marker = if i == sel { "› " } else { "  " };
        let (value, style) = match ed.notify.get(label) {
            Some(s @ (NotifyState::Alarm | NotifyState::Emergency)) => {
                (s.as_str(), Style::new().fg(Color::Red))
            }
            Some(s) => (s.as_str(), Style::new().fg(Color::Yellow)),
            None => ("normal", Style::new().fg(Color::DarkGray)),
        };
        let label_style = if i == sel {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        body.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(format!("{label:<20}"), label_style),
            Span::styled(value, style),
        ]));
    }
    draw_modal(
        f,
        area,
        w,
        &body,
        format!(" Notifications for {} ", field_id_tag(ed.field)),
        " Space cycle · a alarm · w warn · e emergency · n normal · Enter save · Esc back ",
    );
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
            // In mapping mode each device carries how many of its fields
            // publish, so an unmapped device is visible without opening it.
            let mut spans = vec![
                Span::styled(format!("{sym} "), Style::new().fg(color)),
                Span::raw(label),
            ];
            if let Some(n) = app.mapped_count(id) {
                spans.push(Span::styled(
                    format!(" [{n}]"),
                    Style::new().fg(if n == 0 {
                        Color::DarkGray
                    } else {
                        Color::Green
                    }),
                ));
            }
            Line::from(spans).into()
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
    // A device-reference (`DeviceList`) field only indexes the *full* bus device
    // list when it is an **event target** — the vendor library builds a
    // per-field, filtered candidate list for other device-reference fields (e.g.
    // a battery's `Cluster master`, which points into the battery cluster), and
    // that filter isn't reversed yet. An event target is identifiable
    // structurally: a `DeviceList` field immediately followed by an
    // `EventCommand`. Only those are resolved to a device name; other
    // device-reference fields show their raw index rather than a wrong device.
    let next_field_viz = |from: usize| -> Option<VisualizationType> {
        app.rows[from + 1..].iter().find_map(|r| match r {
            Row::Field(f) => Some(f.viz_type),
            Row::Group(_) => None,
        })
    };
    // `Event N command` selects an output of the event's target device (the
    // preceding target field); track the last event-target device id for it.
    let mut last_target: Option<u32> = None;
    let mut items: Vec<ListItem> = Vec::with_capacity(app.rows.len());
    for (i, row) in app.rows.iter().enumerate() {
        let item = match row {
            Row::Group(name) => ListItem::new(Span::styled(
                name.clone(),
                Style::new().add_modifier(Modifier::BOLD).fg(Color::Yellow),
            )),
            Row::Field(field) => {
                let value = app.values.get(&field.index);
                let is_event_target = field.viz_type == VisualizationType::DeviceList
                    && next_field_viz(i) == Some(VisualizationType::EventCommand);
                if is_event_target {
                    last_target = match value {
                        Some(Value::DeviceRef { index, .. }) => {
                            app.device_ids.get(*index as usize).copied()
                        }
                        _ => None,
                    };
                }
                let val = match (field.viz_type, value) {
                    // Event target: resolve against the full bus device list.
                    (VisualizationType::DeviceList, Some(Value::DeviceRef { index, .. })) => {
                        if is_event_target {
                            device_ref_label(*index, &app.device_ids, &names)
                        } else {
                            format!("[{index}]")
                        }
                    }
                    // Command: resolve the index to the target's output name.
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
                let head = format!(
                    "  {} {:<22} {rw} {val:>VALUE_COL$} {:<4}",
                    field_id_tag(field.index),
                    truncate(&field.name, 22),
                    field.unit,
                );
                // In mapping mode, show where this field publishes. Signal K
                // paths are long and the columns to their left are already
                // wide, so on a narrow terminal the path wraps onto a
                // continuation line rather than being cut off — the path is the
                // whole point of the mode, and a truncated one is unreadable.
                match app.mapped_path(field.index) {
                    Some(path) => mapped_item(head, path, content.width as usize),
                    None if app.mapping_mode() => ListItem::new(Line::from(vec![
                        Span::raw(head),
                        Span::styled(" \u{2014}", Style::new().fg(Color::DarkGray)),
                    ])),
                    None => ListItem::new(Line::raw(head)),
                }
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
/// canonical order [`masterbus::MasterBus::devices_all`] returns (see FINDINGS)
/// — so
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

/// Indent of a wrapped Signal K path, including its continuation marker.
const WRAP_INDENT: &str = "      \u{21b3} ";

/// One field row in mapping mode: the field's columns, then its Signal K path
/// on the same line when it fits, otherwise on a continuation line.
///
/// `avail` is the width of the list's content area. A `ListItem` may be several
/// lines tall and the selection highlight covers all of them, so wrapping costs
/// nothing but vertical space.
fn mapped_item<'a>(head: String, path: &str, avail: usize) -> ListItem<'a> {
    ListItem::new(mapped_lines(head, path, avail))
}

/// The one or two lines of a mapping row. Split out from [`mapped_item`] so the
/// wrapping decision can be tested without rendering a frame.
fn mapped_lines<'a>(head: String, path: &str, avail: usize) -> Vec<Line<'a>> {
    let green = Style::new().fg(Color::Green);
    if head.chars().count() + 1 + path.chars().count() <= avail {
        return vec![Line::from(vec![
            Span::raw(head),
            Span::styled(format!(" {path}"), green),
        ])];
    }
    // Still too long for a line of its own: truncate rather than wrap twice.
    let room = avail.saturating_sub(WRAP_INDENT.chars().count());
    vec![
        Line::raw(head),
        Line::from(Span::styled(
            format!("{WRAP_INDENT}{}", truncate(path, room)),
            green,
        )),
    ]
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
mod mapping_row_tests {
    use super::*;

    #[test]
    fn modal_rows_follow_the_wrapped_width() {
        assert_eq!(wrapped_rows(0, 40), 1);
        assert_eq!(wrapped_rows(40, 40), 1);
        assert_eq!(wrapped_rows(41, 40), 2);
        assert_eq!(wrapped_rows(120, 40), 3);
        assert_eq!(wrapped_rows(10, 0), 1);
    }

    /// The columns to the left of the path are fixed-width and already wide, so
    /// this is what a real row's head looks like.
    fn head() -> String {
        format!(
            "  {} {:<22} {:<2} {:>VALUE_COL$} {:<4}",
            field_id_tag(0x001),
            "Battery",
            "ro",
            "26.35",
            "V"
        )
    }

    /// Flatten a line back to the text a terminal would show.
    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    const PATH: &str = "electrical.batteries.main-batt.voltage";

    #[test]
    fn a_path_that_fits_stays_on_one_line() {
        let h = head();
        let exact = h.chars().count() + 1 + PATH.chars().count();
        let l = mapped_lines(h.clone(), PATH, exact);
        assert_eq!(l.len(), 1);
        assert_eq!(text(&l[0]), format!("{h} {PATH}"));
        // One column of slack is still one line.
        assert_eq!(mapped_lines(h, PATH, exact + 1).len(), 1);
    }

    #[test]
    fn a_path_one_column_too_wide_wraps() {
        let h = head();
        let exact = h.chars().count() + 1 + PATH.chars().count();
        let l = mapped_lines(h.clone(), PATH, exact - 1);
        assert_eq!(l.len(), 2);
        // The head is untouched, so the columns stay aligned with the rows
        // above and below.
        assert_eq!(text(&l[0]), h);
        // The whole path survives on the continuation line.
        assert!(text(&l[1]).ends_with(PATH), "{}", text(&l[1]));
        assert!(text(&l[1]).starts_with(WRAP_INDENT));
    }

    /// A terminal narrow enough that even the continuation line cannot hold the
    /// path must not wrap a second time; it truncates instead.
    #[test]
    fn a_very_narrow_pane_truncates_rather_than_wrapping_twice() {
        let l = mapped_lines(head(), PATH, 30);
        assert_eq!(l.len(), 2);
        let cont = text(&l[1]);
        assert!(cont.chars().count() <= 30, "{} chars", cont.chars().count());
        assert!(cont.ends_with('\u{2026}'), "{cont}");
    }

    /// Degenerate width must not panic or produce a negative-width slice.
    #[test]
    fn an_absurdly_narrow_pane_is_survivable() {
        for w in [0usize, 1, 5, 8] {
            let l = mapped_lines(head(), PATH, w);
            assert_eq!(l.len(), 2);
        }
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

/// Formatting rules for the value column, independent of any frame.
#[cfg(test)]
mod format_tests {
    use super::*;
    use masterbus::{Date, Time};

    #[test]
    fn numbers_booleans_and_text_have_fixed_shapes() {
        assert_eq!(format_value(&Value::Float(26.3456)), "26.35");
        assert_eq!(format_value(&Value::Float(-0.004)), "-0.00");
        assert_eq!(format_value(&Value::Boolean(true)), "on");
        assert_eq!(format_value(&Value::Boolean(false)), "off");
        assert_eq!(
            format_value(&Value::Text {
                sid: 1,
                text: "Nav Chg".into()
            }),
            "Nav Chg"
        );
        assert_eq!(format_value(&Value::Invalid), "invalid");
    }

    /// The device reports "no value" as a NaN float or a negative date/time
    /// component; all three render as one em dash rather than as noise.
    #[test]
    fn the_no_value_sentinels_all_render_as_a_dash() {
        assert_eq!(format_value(&Value::Float(f32::NAN)), "—");
        assert_eq!(
            format_value(&Value::Date(Date {
                day: -1,
                mon: -1,
                year: -1
            })),
            "—"
        );
        assert_eq!(
            format_value(&Value::Time(Time {
                sec: -1,
                min: 0,
                hour: 0,
                days: 0
            })),
            "—"
        );
    }

    #[test]
    fn dates_and_durations_are_zero_padded() {
        assert_eq!(
            format_value(&Value::Date(Date {
                day: 7,
                mon: 5,
                year: 2026
            })),
            "2026-05-07"
        );
        assert_eq!(
            format_value(&Value::Time(Time {
                sec: 5,
                min: 4,
                hour: 3,
                days: 2
            })),
            "2d 03:04:05"
        );
    }

    /// A list index with no label falls back to the bare index, so an
    /// undiscovered enum still shows what the device actually reports.
    #[test]
    fn a_list_without_labels_shows_its_index() {
        let with = Value::List {
            index: 1,
            options: vec!["Off".into(), "On".into()],
        };
        assert_eq!(format_value(&with), "On");

        let without = Value::List {
            index: 3,
            options: vec!["Off".into()],
        };
        assert_eq!(format_value(&without), "[3]");

        let eventable = Value::Eventable {
            index: 9,
            labels: vec![],
        };
        assert_eq!(format_value(&eventable), "[9]");
    }

    /// In the field pane a list value is shown as `label(index)`, so the wire
    /// value stays visible next to its meaning. The labels may come from the
    /// value or, when it carries none, from the field's schema.
    #[test]
    fn list_values_show_their_label_and_their_index() {
        let devices = [];
        let names = HashMap::new();
        let schema = vec!["Off".into(), "On".into(), "Auto".into()];

        let from_value = Value::List {
            index: 2,
            options: vec!["A".into(), "B".into(), "C".into()],
        };
        assert_eq!(
            format_value_for(&from_value, &schema, &devices, &names),
            "C(2)"
        );

        let from_schema = Value::List {
            index: 2,
            options: vec![],
        };
        assert_eq!(
            format_value_for(&from_schema, &schema, &devices, &names),
            "Auto(2)"
        );

        let unknown = Value::List {
            index: 7,
            options: vec![],
        };
        assert_eq!(
            format_value_for(&unknown, &schema, &devices, &names),
            "[7](7)"
        );
    }

    /// An out-of-range device reference shows the raw index — the target may
    /// simply be powered off right now.
    #[test]
    fn an_out_of_range_device_reference_shows_its_index() {
        let names = HashMap::new();
        assert_eq!(device_ref_label(9, &[0x188EA2], &names), "→ [9]");
        assert_eq!(device_ref_label(-1, &[0x188EA2], &names), "→ [-1]");
    }

    #[test]
    fn every_device_status_has_a_glyph() {
        use DeviceStatus as S;
        for s in [
            S::On,
            S::OnWarning,
            S::Sleeping,
            S::OffFault,
            S::OffError,
            S::Updating,
            S::Offline,
            S::Unknown,
        ] {
            let (glyph, _) = status_style(s);
            assert_eq!(glyph.chars().count(), 1, "{s:?}");
        }
        assert_eq!(status_style(S::On).0, "●");
        assert_eq!(status_style(S::Offline).0, "○");
        assert_eq!(status_style(S::Unknown).0, "?");
    }

    /// Truncation counts characters, not bytes, and the ellipsis is part of
    /// the budget — a column must never overflow into its neighbour.
    #[test]
    fn truncation_fits_the_budget_including_the_ellipsis() {
        assert_eq!(truncate("Voltage", 16), "Voltage");
        assert_eq!(truncate("Voltage", 7), "Voltage");
        assert_eq!(truncate("Voltage", 4), "Vol…");
        assert_eq!(truncate("Voltage", 1), "…");
        assert_eq!(truncate("Voltage", 0), "…");
        // Multi-byte input is measured in characters.
        assert_eq!(truncate("Temperatuur °C", 5).chars().count(), 5);
    }

    /// The field-id tag is the same three-hex-digit form `masterbus-set-field`
    /// takes on the command line, with the channel in bit 8.
    #[test]
    fn field_id_tags_carry_the_channel_bit() {
        assert_eq!(field_id_tag(masterbus::field_id::btm1(0x17)), "0x017");
        assert_eq!(field_id_tag(masterbus::field_id::btm3(0x17)), "0x117");
        assert_eq!(field_id_tag(masterbus::field_id::btm3(0xFF)), "0x1FF");
    }
}

/// Whole-frame rendering, through ratatui's test backend. These assert on
/// what a user would actually see on the screen: which pane is drawn, what
/// the modals say, and that no layout panics at awkward sizes.
#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::app::app_tests::{ADDR, app, field};
    use crate::app::{EditKind, Editor, LoginPrompt, LoginStage, ValuesView};
    use masterbus::{Menu, Value, field_id};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    /// Render the app and return the screen as text, one line per row.
    fn screen_at(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        term.draw(|f| draw(f, app)).expect("draw");
        text(term.backend().buffer())
    }

    fn screen(app: &App) -> String {
        screen_at(app, 100, 30)
    }

    fn text(buf: &Buffer) -> String {
        let area = buf.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Put a discovered-looking field pane in place without running discovery.
    fn with_rows(app: &mut App) {
        app.cur_device = Some(ADDR);
        app.focus = Focus::Fields;
        app.cur_tab = TabKind::Menu(Menu::Monitoring);
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
        app.values.insert(field_id::btm1(0x17), Value::Float(26.35));
    }

    #[test]
    fn the_device_list_is_drawn_with_names_and_status() {
        let (app, _bus) = app();
        app.names.lock().unwrap().insert(ADDR, "Combi".into());
        let s = screen(&app);
        assert!(s.contains("Combi"), "{s}");
        // The live-status glyph for a device that is broadcasting.
        assert!(s.contains('●'), "{s}");
    }

    /// The footer always carries the current status line, which is how the
    /// TUI reports what just happened.
    #[test]
    fn the_footer_shows_the_status_line() {
        let (mut app, _bus) = app();
        app.status = "set failed: field is read-only".into();
        assert!(screen(&app).contains("set failed: field is read-only"));
    }

    #[test]
    fn opening_a_device_draws_the_tab_bar_and_summary() {
        let (mut app, _bus) = app();
        app.open_device();
        let s = screen(&app);
        assert!(s.contains("Summary"), "{s}");
        assert!(s.contains(&tab_label(TABS[1])), "{s}");
    }

    #[test]
    fn the_field_pane_lists_groups_fields_and_values() {
        let (mut app, _bus) = app();
        with_rows(&mut app);
        let s = screen(&app);
        assert!(s.contains("DC"), "{s}");
        assert!(s.contains("Voltage"), "{s}");
        assert!(s.contains("Current"), "{s}");
        assert!(s.contains("26.35"), "{s}");
        // The read-only field is marked as such.
        assert!(s.contains("ro"), "{s}");
    }

    #[test]
    fn a_discovery_in_flight_shows_a_spinner_and_what_it_is_doing() {
        let (mut app, _bus) = app();
        app.names.lock().unwrap().insert(ADDR, "Combi".into());
        app.open_device();
        app.next_tab(); // switching to a data tab spawns the discovery worker
        assert!(app.discovering());
        let s = screen(&app);
        assert!(
            SPINNER.iter().any(|c| s.contains(*c)),
            "expected a spinner frame:\n{s}"
        );
        assert!(s.contains("Combi"), "{s}");
    }

    #[test]
    fn the_edit_modal_shows_the_field_and_the_buffer() {
        let (mut app, _bus) = app();
        with_rows(&mut app);
        app.editor = Some(Editor {
            field: field_id::btm1(0x17),
            name: "Voltage".into(),
            kind: EditKind::Number("13.2".into()),
        });
        let s = screen(&app);
        assert!(s.contains("Voltage"), "{s}");
        assert!(s.contains("13.2"), "{s}");
    }

    #[test]
    fn the_choice_editor_lists_its_options() {
        let (mut app, _bus) = app();
        with_rows(&mut app);
        app.editor = Some(Editor {
            field: field_id::btm1(0x05),
            name: "Mode".into(),
            kind: EditKind::Choice {
                options: vec!["Off".into(), "On".into(), "Auto".into()],
                sel: 2,
            },
        });
        let s = screen(&app);
        assert!(s.contains("Mode"), "{s}");
        assert!(s.contains("Auto"), "{s}");
    }

    #[test]
    fn the_values_modal_lists_every_option() {
        let (mut app, _bus) = app();
        with_rows(&mut app);
        app.values_modal = Some(ValuesView {
            field_name: "Mode".into(),
            options: vec!["Off".into(), "On".into(), "Auto".into()],
            current: Some(1),
        });
        let s = screen(&app);
        assert!(s.contains("Mode"), "{s}");
        for opt in ["Off", "On", "Auto"] {
            assert!(s.contains(opt), "missing {opt}:\n{s}");
        }
    }

    /// The login modal walks two stages: pick a level, then type a code. The
    /// code must not be echoed back to the screen.
    #[test]
    fn the_login_modal_shows_the_levels_then_masks_the_code() {
        let (mut app, _bus) = app();
        app.login = Some(LoginPrompt {
            device: ADDR,
            sel: 1,
            current: Some(masterbus::AccessLevel::EndUser),
            stage: LoginStage::PickLevel,
        });
        let s = screen(&app);
        for level in LOGIN_LEVELS {
            assert!(s.contains(level_label(level)), "missing {level:?}:\n{s}");
        }

        app.login = Some(LoginPrompt {
            device: ADDR,
            sel: 1,
            current: Some(masterbus::AccessLevel::EndUser),
            stage: LoginStage::EnterPassword {
                level: masterbus::AccessLevel::Installer,
                buf: "1234".into(),
            },
        });
        let s = screen(&app);
        assert!(s.contains("password:"), "{s}");
        assert!(!s.contains("1234"), "the code must not be echoed:\n{s}");
        // Four typed characters, four bullets.
        assert!(s.contains("••••"), "{s}");
    }

    /// The log pane is only drawn when logging is routed into the TUI.
    #[test]
    fn the_log_pane_is_drawn_only_when_enabled() {
        let (mut app, _bus) = app();
        let without = screen(&app);
        app.logs_in_tui = true;
        app.show_logs = true;
        let with = screen(&app);
        assert_ne!(without, with, "the log pane should change the layout");
    }

    /// Rendering must survive a terminal far smaller than the layout wants —
    /// a panic here would take the whole TUI down on a resize.
    #[test]
    fn rendering_survives_an_absurdly_small_terminal() {
        let (mut app, _bus) = app();
        with_rows(&mut app);
        app.editor = Some(Editor {
            field: field_id::btm1(0x17),
            name: "Voltage".into(),
            kind: EditKind::Number("13.2".into()),
        });
        for (w, h) in [(1, 1), (4, 3), (20, 5), (40, 10)] {
            let _ = screen_at(&app, w, h);
        }
    }
}
