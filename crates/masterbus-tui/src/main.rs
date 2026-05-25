//! Terminal UI for browsing and editing MasterBus devices.
//!
//! Usage:
//!   masterbus-tui <can-iface> [cache-dir]    # Linux SocketCAN
//!   masterbus-tui usb [serial] [cache-dir]   # explicit USB link (any OS)
//!   masterbus-tui [serial] [cache-dir]       # macOS/Windows: USB link (no arg needed)
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bus = connect(&args)?;
    println!("connected; scanning the bus…");
    run_tui(bus)?;
    Ok(())
}

/// On Linux both SocketCAN and the USB link are available, so a transport must be
/// chosen: the first argument is a CAN interface, or `usb [serial]` for the link.
#[cfg(target_os = "linux")]
fn connect(args: &[String]) -> Result<MasterBus, Box<dyn std::error::Error>> {
    let Some(first) = args.first() else {
        eprintln!("usage: masterbus-tui <can-iface> [cache-dir]");
        eprintln!("       masterbus-tui usb [serial] [cache-dir]");
        std::process::exit(1);
    };
    if first == "usb" {
        let config = Config { cache_path: args.get(2).map(Into::into), ..Default::default() };
        println!("masterbus-tui: connecting to USB link…");
        Ok(MasterBus::usb(args.get(1).map(String::as_str), config)?)
    } else {
        let config = Config { cache_path: args.get(1).map(Into::into), ..Default::default() };
        println!("masterbus-tui: connecting to {first}…");
        Ok(MasterBus::socketcan(first, config)?)
    }
}

/// Off Linux the USB link is the only transport, so no interface argument is
/// needed; an optional first argument selects a specific link by serial number.
#[cfg(not(target_os = "linux"))]
fn connect(args: &[String]) -> Result<MasterBus, Box<dyn std::error::Error>> {
    let config = Config { cache_path: args.get(1).map(Into::into), ..Default::default() };
    println!("masterbus-tui: connecting to USB link…");
    Ok(MasterBus::usb(args.first().map(String::as_str), config)?)
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
