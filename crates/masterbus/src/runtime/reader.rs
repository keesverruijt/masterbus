//! Reader thread: drains the transport, routes discovery responses to the
//! waiter, caches values, and tracks device liveness.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::Sender;

use super::state::State;
use super::waiter::Waiter;
use super::{Config, DeviceEvent};
use crate::protocol::{decode_value, frame_from_raw, parse_frame, waiter_key_for_frame, MbMessage, SHADOW_BIT};
use crate::transport::TransportRx;

/// Build the value-waiter key for an on-demand read/poll response.
pub(super) fn value_key(addr: u32, field: i32) -> String {
    format!("val:{:06X}:{}", addr, field)
}

pub(super) fn spawn(
    mut rx: Box<dyn TransportRx>,
    state: Arc<State>,
    waiter: Arc<Waiter>,
    dev_tx: Sender<DeviceEvent>,
    shutdown: Arc<AtomicBool>,
    config: Config,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("masterbus-reader".into())
        .spawn(move || reader_loop(&mut *rx, &state, &waiter, &dev_tx, &shutdown, &config))
        .expect("spawn reader")
}

fn reader_loop(
    rx: &mut dyn TransportRx,
    state: &State,
    waiter: &Waiter,
    dev_tx: &Sender<DeviceEvent>,
    shutdown: &AtomicBool,
    config: &Config,
) {
    let mut alive: HashSet<u32> = HashSet::new();
    while !shutdown.load(Ordering::Relaxed) {
        match rx.recv(Duration::from_millis(200)) {
            Ok(Some((raw_id, data))) => handle_frame(raw_id, &data, state, waiter),
            Ok(None) => {}
            Err(_) => {
                // transient transport error; keep going
            }
        }

        // Liveness diff → presence events.
        let now: HashSet<u32> = state.alive_ids(config.liveness).into_iter().collect();
        for &a in now.difference(&alive) {
            let _ = dev_tx.send(DeviceEvent::Alive(a));
        }
        for &a in alive.difference(&now) {
            let _ = dev_tx.send(DeviceEvent::Offline(a));
        }
        alive = now;
    }
    waiter.cancel_all();
}

fn handle_frame(raw_id: u32, data: &[u8], state: &State, waiter: &Waiter) {
    let frame = frame_from_raw(raw_id, data);

    // Discovery responses (property / schema / shadow) → matching waiter.
    if let Some(key) = waiter_key_for_frame(frame.can_class, frame.device_addr, data) {
        waiter.deliver(&key, data.to_vec());
    }

    match parse_frame(&frame) {
        MbMessage::DeviceBroadcast { device_addr, type_code, firmware_version, .. } => {
            state.mark_alive(device_addr, type_code, firmware_version);
        }
        MbMessage::MonitoringData { device_addr, field_index, raw, .. }
            if device_addr & SHADOW_BIT == 0 =>
        {
            state.touch(device_addr);
            // Wake any pending on-demand read/poll for this field.
            waiter.deliver(&value_key(device_addr, field_index as i32), raw.clone());
            // Cache the decoded value if we know the field's type.
            if let Some(schema) = state.schema(device_addr)
                && let Some(f) = schema.field(field_index as i32)
            {
                // Carry the schema's option labels so callers get both the
                // numeric index and its meaning.
                let v = decode_value(&raw, f.viz_type).with_options(&f.options);
                state.put_value(device_addr, field_index as i32, v);
            }
        }
        _ => {}
    }
}
