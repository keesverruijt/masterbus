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

mod app;
mod ui;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use app::{App, Focus, Names};
use masterbus::{Config, MasterBus};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logger();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("usage: masterbus-tui");
        eprintln!("transport, heartbeat-master role, and schema cache come from");
        eprintln!("the config file (see `masterbus::FileConfig`)");
        eprintln!();
        eprintln!("logs are written to $MASTERBUS_TUI_LOG (default: ./masterbus-tui.log)");
        eprintln!("filter via RUST_LOG, e.g. RUST_LOG=masterbus=debug");
        return Ok(());
    }
    let bus = MasterBus::auto(Config::default())?;
    println!("connected; scanning the bus…");
    run_tui(bus)?;
    Ok(())
}

/// Send `log` output to a file so it doesn't corrupt the alt-screen TUI.
/// Default level is `warn` — set `RUST_LOG=masterbus=debug` (etc.) to widen.
fn init_logger() {
    let path = std::env::var_os("MASTERBUS_TUI_LOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("masterbus-tui.log"));
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else {
        // Couldn't open the log file — silently disable logging rather than
        // corrupting the UI; the TUI runs without any logs in that case.
        return;
    };
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Pipe(Box::new(file)))
        .try_init();
}

fn run_tui(bus: MasterBus) -> std::io::Result<()> {
    let device_events = bus.device_events();
    let names: Names = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    spawn_name_backfill(bus.clone(), names.clone(), stop.clone());

    let mut app = App::new(bus, names);
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
fn spawn_name_backfill(bus: MasterBus, names: Names, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            for dev in bus.devices() {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let id = dev.id();
                if names.lock().unwrap().contains_key(&id) {
                    continue;
                }
                if let Ok(name) = dev.name()
                    && !name.is_empty()
                {
                    names.lock().unwrap().insert(id, name);
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
}

fn spawn_key_reader() -> Receiver<KeyEvent> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || loop {
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
    });
    rx
}

fn handle_key(app: &mut App, key: KeyEvent) {
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
        KeyCode::Char('q') => app.quit(),
        KeyCode::Char('l') | KeyCode::Char('L') => app.open_login(),
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
                KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => app.back_to_devices(),
                _ => {}
            },
        },
    }
}
