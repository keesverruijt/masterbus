//! Terminal UI for browsing and editing MasterBus devices.
//!
//! Usage: `masterbus-tui <can-iface> [cache-dir]` (Linux/SocketCAN).
//!
//! Left pane: devices (with liveness). Right pane: the selected device's groups
//! and fields with live monitoring values. Writable fields can be edited:
//! booleans toggle, numbers open a text editor, lists cycle with ←/→.

// The TUI runtime is only reachable on Linux (SocketCAN); on other hosts the
// modules type-check but are unused.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod app;
mod ui;

use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use app::{App, Focus};
use masterbus::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let iface = match args.next() {
        Some(s) => s,
        None => {
            eprintln!("usage: masterbus-tui <can-iface> [cache-dir]");
            std::process::exit(1);
        }
    };
    let cache = args.next();
    let config = Config { cache_path: cache.map(Into::into), ..Default::default() };

    #[cfg(target_os = "linux")]
    {
        let bus = masterbus::MasterBus::socketcan(&iface, config)?;
        run_tui(bus)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (iface, config);
        eprintln!("masterbus-tui requires Linux/SocketCAN");
    }
    Ok(())
}

fn run_tui(bus: masterbus::MasterBus) -> std::io::Result<()> {
    let device_events = bus.device_events();
    let mut app = App::new(bus);
    let keys = spawn_key_reader();

    let mut terminal = ratatui::init();
    let result = loop {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &app)) {
            break Err(e);
        }
        if let Ok(key) = keys.recv_timeout(Duration::from_millis(100)) {
            handle_key(&mut app, key);
        }
        while let Ok(ev) = device_events.try_recv() {
            if let masterbus::DeviceEvent::Alive(id) = ev {
                app.note_alive(id);
            }
        }
        app.pump_subscription();
        if app.should_quit {
            break Ok(());
        }
    };
    ratatui::restore();
    result
}

fn spawn_key_reader() -> Receiver<KeyEvent> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || loop {
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => {
                if let Ok(Event::Key(k)) = event::read() {
                    if k.kind == KeyEventKind::Press && tx.send(k).is_err() {
                        break;
                    }
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

    match key.code {
        KeyCode::Char('q') => app.quit(),
        _ => match app.focus {
            Focus::Devices => match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_device(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_device(1),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.open_device(),
                _ => {}
            },
            Focus::Fields => match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_row(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_row(1),
                KeyCode::Enter | KeyCode::Char('e') => app.begin_edit(),
                KeyCode::Char('r') => app.reread_selected(),
                KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => app.back_to_devices(),
                _ => {}
            },
        },
    }
}
