//! Terminal UI for browsing and editing MasterBus devices.
//!
//! Usage:
//!   masterbus-tui [cache-dir]
//!
//! Transport (USB or SocketCAN) and master role are read from the per-host
//! config file (see `masterbus::FileConfig`); the file is created on first run.
//!
//! Left pane: devices (with liveness). Right pane: the selected device's groups
//! and fields with live monitoring values. Writable fields can be edited:
//! booleans toggle, numbers open a text editor, lists cycle with ←/→.
//!
//! # Mapping editor
//!
//! `masterbus-tui --mapping` additionally edits `mapping.json`, the file that
//! decides what `masterbus-signalk` publishes (see `masterbus_tools::mapping`).
//! The device list shows how many of each device's fields are mapped, the
//! Monitoring tab gains a Signal K column, and `+` / `-` add and remove a
//! mapping on the selected field. `+` pre-fills a suggestion; `a` copies the
//! open device's mapping to every other device with the same article; `w`
//! writes the file.

mod app;
mod ui;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use app::{App, Focus, Idents, MappingSession, Names};
use masterbus::{Config, MasterBus};
use masterbus_tools::mapping::Mapping;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let logging_in_tui = init_logger();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("usage: masterbus-tui [--mapping]");
        eprintln!();
        eprintln!("  --mapping   edit the Signal K mapping file alongside browsing:");
        eprintln!("              + map the selected field, - unmap, a apply to every");
        eprintln!("              device with the same article, w write the file.");
        eprintln!();
        eprintln!("transport, heartbeat-master role, and schema cache come from");
        eprintln!("the config file (see `masterbus::FileConfig`)");
        eprintln!();
        eprintln!("logs go to an in-TUI pane (toggle with `~`), level `info`.");
        eprintln!("set MASTERBUS_TUI_LOG=<path> to redirect to a file instead.");
        return Ok(());
    }
    let want_mapping = args.iter().any(|a| a == "--mapping");

    // Resolve and load the mapping before taking over the terminal, so a
    // malformed file is reported plainly rather than behind a TUI.
    let mapping = if want_mapping {
        let path = std::env::var_os("MAPPING")
            .map(std::path::PathBuf::from)
            .map_or_else(
                || masterbus::FileConfig::load_or_create().map(|c| c.mapping_path()),
                Ok,
            )?;
        // A hand-edited file is the normal case, so a syntax error has to read
        // like a syntax error rather than a debug-printed io::Error.
        let map = match Mapping::load(&path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("masterbus-tui: {e}");
                eprintln!("fix the file, or move it aside to start over.");
                std::process::exit(2);
            }
        };
        println!("mapping: {} ({} field(s))", path.display(), map.len());
        Some(MappingSession {
            path,
            map,
            dirty: false,
            quit_armed: false,
        })
    } else {
        None
    };

    let bus = MasterBus::auto(Config::default())?;
    println!("connected; scanning the bus…");
    run_tui(bus, mapping, logging_in_tui)?;
    Ok(())
}

/// Pick a destination for the `log` facade output:
///
/// - If `$MASTERBUS_TUI_LOG` is set, append to that file (silent on the UI).
/// - Otherwise route through `tui-logger`, which the TUI displays in a
///   toggleable bottom pane.
///
/// Returns whether logs land in the in-TUI pane (so the UI knows to render it).
fn init_logger() -> bool {
    if let Some(path) = std::env::var_os("MASTERBUS_TUI_LOG") {
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::path::PathBuf::from(path))
        {
            let _ =
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
                    .target(env_logger::Target::Pipe(Box::new(file)))
                    .try_init();
        }
        return false;
    }
    // `init_logger` here is `tui_logger::init_logger`, not ours.
    let _ = tui_logger::init_logger(log::LevelFilter::Trace);
    tui_logger::set_default_level(log::LevelFilter::Info);
    true
}

fn run_tui(
    bus: MasterBus,
    mapping: Option<MappingSession>,
    logs_in_tui: bool,
) -> std::io::Result<()> {
    let device_events = bus.device_events();
    let names: Names = Arc::new(Mutex::new(HashMap::new()));
    let idents: Idents = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    spawn_name_backfill(bus.clone(), names.clone(), idents.clone(), stop.clone());

    let mut app = App::new(bus, names, idents, mapping, logs_in_tui);
    let keys = spawn_key_reader();

    let mut terminal = ratatui::init();
    let result = loop {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &app)) {
            break Err(e);
        }
        if let Ok(key) = keys.recv_timeout(Duration::from_millis(100)) {
            handle_key(&mut app, key);
        }
        app.tick = app.tick.wrapping_add(1);
        while let Ok(ev) = device_events.try_recv() {
            if let masterbus::DeviceEvent::Alive(id) = ev {
                app.note_alive(id);
            }
        }
        app.poll_pending();
        app.pump_subscription();
        if app.should_quit {
            break Ok(());
        }
    };
    stop.store(true, Ordering::Relaxed);
    ratatui::restore();
    result
}

/// Background thread: resolve device names (cheap identity discovery) as devices
/// appear, so the device list fills in with names over the first seconds.
fn spawn_name_backfill(bus: MasterBus, names: Names, idents: Idents, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            for dev in bus.devices() {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let id = dev.id();
                if idents.lock().unwrap().contains_key(&id) {
                    continue;
                }
                // One identity fetch feeds both maps: `name()` would do the
                // same round trip and throw the rest away, and the mapping
                // editor needs the serial and article.
                if let Ok(ident) = dev.identity() {
                    if !ident.name.is_empty() {
                        names.lock().unwrap().insert(id, ident.name.clone());
                    }
                    idents.lock().unwrap().insert(id, ident);
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
}

fn spawn_key_reader() -> Receiver<KeyEvent> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(200)) {
                Ok(true) => {
                    if let Ok(Event::Key(k)) = event::read()
                        && k.kind == KeyEventKind::Press
                        && tx.send(k).is_err()
                    {
                        break;
                    }
                }
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
    rx
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // The path editor owns every key while open: a Signal K path is free text
    // and may contain any of the letters the browse-mode bindings use.
    if app.path_editing() {
        // The truth-table stage is a small list, not a text field.
        if app.truth_editing() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.truth_move(-1),
                KeyCode::Down | KeyCode::Char('j') => app.truth_move(1),
                KeyCode::Char(' ') => app.truth_toggle(),
                // Before the bare `n` below, which would swallow it.
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.map_editor_toggle_invert()
                }
                KeyCode::Char('t') | KeyCode::Char('y') | KeyCode::Char('1') => app.truth_set(true),
                KeyCode::Char('f') | KeyCode::Char('n') | KeyCode::Char('0') => {
                    app.truth_set(false)
                }
                KeyCode::Enter => app.commit_truth(),
                KeyCode::Esc => app.truth_back(),
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Enter => app.commit_map(),
            KeyCode::Esc => app.cancel_map(),
            KeyCode::Backspace => app.map_editor_backspace(),
            // Ctrl-N toggles the inverted-boolean flag; a bare letter would be
            // swallowed by the text field.
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.map_editor_toggle_invert()
            }
            KeyCode::Char(c) => app.map_editor_char(c),
            _ => {}
        }
        return;
    }

    // Read-only values modal absorbs any key and just closes.
    if app.values_open() {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q') => {
                app.close_values();
            }
            _ => {}
        }
        return;
    }

    if app.editing() {
        match key.code {
            KeyCode::Enter => app.commit_edit(),
            KeyCode::Esc => app.cancel_edit(),
            KeyCode::Backspace => app.editor_backspace(),
            KeyCode::Left => app.editor_choice_move(-1),
            KeyCode::Right => app.editor_choice_move(1),
            KeyCode::Char(c) => app.editor_char(c),
            _ => {}
        }
        return;
    }

    // Login picker owns the keys when open.
    if app.login_modal() {
        if app.login_at_password_stage() {
            match key.code {
                KeyCode::Enter => app.commit_login(),
                KeyCode::Esc => app.cancel_login(),
                KeyCode::Backspace => app.login_backspace(),
                KeyCode::Char(c) => app.login_char(c),
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.login_move(-1),
                KeyCode::Down | KeyCode::Char('j') => app.login_move(1),
                KeyCode::Enter => app.commit_login(),
                KeyCode::Esc => app.cancel_login(),
                _ => {}
            }
        }
        return;
    }

    // While a device is being enumerated, only quit or cancel are allowed.
    if app.discovering() {
        match key.code {
            KeyCode::Char('q') => app.quit(),
            KeyCode::Esc => app.cancel_pending(),
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit_checked(),
        KeyCode::Char('l') | KeyCode::Char('L') => app.open_login(),
        KeyCode::Char('~') => app.toggle_logs(),
        KeyCode::Char('w') if app.mapping_mode() => app.save_mapping(),
        _ => match app.focus {
            Focus::Devices => match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_device(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_device(1),
                KeyCode::Enter | KeyCode::Right => app.open_device(),
                _ => {}
            },
            Focus::Fields => match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_row(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_row(1),
                KeyCode::Tab => app.next_tab(),
                KeyCode::BackTab => app.prev_tab(),
                KeyCode::Enter | KeyCode::Char('e') => app.begin_edit(),
                KeyCode::Char('r') => app.reread_selected(),
                KeyCode::Char('?') => app.open_values(),
                KeyCode::Char('+') | KeyCode::Char('=') if app.mapping_mode() => app.begin_map(),
                KeyCode::Char('-') if app.mapping_mode() => app.unmap_selected(),
                KeyCode::Char('a') if app.mapping_mode() => app.apply_to_article(),
                KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => app.back_to_devices(),
                _ => {}
            },
        },
    }
}
