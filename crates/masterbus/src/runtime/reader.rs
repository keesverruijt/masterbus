//! Reader thread: drains the transport, routes discovery responses to the
//! waiter, caches values, and tracks device liveness.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::Sender;

use super::framelog::frame_log;
use super::state::State;
use super::waiter::Waiter;
use super::{Config, DeviceEvent};
use crate::model::{field_id, DeviceId, FieldId};
use crate::protocol::{
    can_class, decode_value, frame_from_raw, parse_frame, waiter_key_for_frame, MbMessage,
    BTM1_META_ADDR_FLAG,
};
use crate::transport::TransportRx;

/// Build the value-waiter key for an on-demand read/poll response.
pub(super) fn value_key(addr: DeviceId, field: FieldId) -> String {
    format!("val:{:06X}:{:04X}", addr, field)
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
    let mut alive: HashSet<DeviceId> = HashSet::new();
    while !shutdown.load(Ordering::Relaxed) {
        match rx.recv(Duration::from_millis(200)) {
            Ok(Some((raw_id, data))) => handle_frame(raw_id, &data, state, waiter),
            Ok(None) => {}
            Err(_) => {
                // transient transport error; keep going
            }
        }

        // Liveness diff → presence events.
        let now: HashSet<DeviceId> = state.alive_ids(config.liveness).into_iter().collect();
        for &a in now.difference(&alive) {
            log::info!("device 0x{a:06X} alive");
            let _ = dev_tx.send(DeviceEvent::Alive(a));
        }
        for &a in alive.difference(&now) {
            log::info!("device 0x{a:06X} offline");
            let _ = dev_tx.send(DeviceEvent::Offline(a));
        }
        alive = now;
    }
    waiter.cancel_all();
}

fn handle_frame(raw_id: u32, data: &[u8], state: &State, waiter: &Waiter) {
    let frame = frame_from_raw(raw_id, data);
    frame_log("Rx", raw_id, data);

    // Only DEVICE-originated frames register a device: a frame's `device_addr`
    // is the addressed device for master→device requests (class `0x05`/`0x07`/
    // `0x18`/`0x19`/`0x1A`/`0x1B`/`0x1C`), but the SOURCE device for the
    // response/push/broadcast classes. SocketCAN loops our own outbound
    // requests back as `Up` frames, so without this filter we'd register
    // ourselves (when running as bus master, class `0x05` from our address)
    // and any other master polling the bus. Real devices stay reachable
    // because they emit broadcasts / responses / pushes on the device
    // classes. Mask the Btm1-meta shadow flag so `0xBA3B4B` and `0x3A3B4B`
    // share one entry.
    if is_device_originated(frame.can_class) {
        state.touch(frame.device_addr & !BTM1_META_ADDR_FLAG);
    }

    // Discovery responses (property / schema / per-field metadata) → matching waiter.
    if let Some(key) = waiter_key_for_frame(frame.can_class, frame.device_addr, data) {
        waiter.deliver(&key, data.to_vec());
    }

    match parse_frame(&frame) {
        MbMessage::DeviceBroadcast { device_addr, type_code, firmware_version, .. } => {
            state.mark_alive(device_addr, type_code, firmware_version);
        }
        MbMessage::MonitoringData { device_addr, field_index, raw, .. }
            if device_addr & BTM1_META_ADDR_FLAG == 0 =>
        {
            // Monitoring data arrives on the Btm1 channel: build a Btm1
            // `FieldId` from the wire field index.
            let field = field_id::btm1(field_index);
            state.touch(device_addr);
            // Wake any pending on-demand read/poll for this field.
            waiter.deliver(&value_key(device_addr, field), raw.clone());
            // Cache the decoded value if we know the field's type — look in
            // both the menu-grouped schema and the flat Btm3 list.
            if let Some(f) = state.field_info(device_addr, field) {
                let v = decode_value(&raw, f.viz_type).with_options(&f.options);
                state.put_value(device_addr, field, v);
            }
        }
        _ => {
            // Btm3 live-value carrier: class `0x0B` on `addr | 0x800000`
            // with a headerless 6-byte payload `[fid_lo, fid_hi, b0..b3]`.
            // Same frame shape serves both unsolicited pushes *and*
            // write-acks; both populate the value cache and wake any
            // pending `poll_value` waiter via the channel-tagged value
            // key. See PROTOCOL.md §6 / FINDINGS §3e+§3f.
            if frame.can_class == can_class::SCHEMA_DATA_HISTORY
                && frame.device_addr & BTM1_META_ADDR_FLAG != 0
                && data.len() >= 6
            {
                let real = frame.device_addr & !BTM1_META_ADDR_FLAG;
                // The Btm3 channel uses an 8-bit wire index in our
                // `FieldId` encoding; the on-the-wire high byte has been
                // 0 on every device seen so far.
                let field = field_id::btm3(data[0]);
                let raw_value = data[2..6].to_vec();
                state.touch(real);
                waiter.deliver(&value_key(real, field), raw_value.clone());
                if let Some(f) = state.field_info(real, field) {
                    let v = decode_value(&raw_value, f.viz_type).with_options(&f.options);
                    state.put_value(real, field, v);
                }
            }
        }
    }
}

/// Whether a CAN class is emitted by a *device* (broadcast / response / push)
/// rather than by a master (request / heartbeat). Used by [`handle_frame`] to
/// gate auto-registration of devices, so that our own loopback and any other
/// master polling the bus don't appear as spurious entries in the device
/// list. See PROTOCOL.md §3 for the class taxonomy.
fn is_device_originated(can_class: u8) -> bool {
    matches!(
        can_class,
        can_class::DEVICE_BROADCAST           // 0x04 — periodic self-announce
        | can_class::PROPERTY_INFO            // 0x06 — reply to class-0x07 request
        | can_class::MONITORING_DATA          // 0x08 — Btm1 value / Btm1 metadata reply
        | can_class::SCHEMA_DATA              // 0x09 — schema reply (Monitoring)
        | can_class::SCHEMA_DATA_ALARM        // 0x0A — schema reply (Alarms)
        | can_class::SCHEMA_DATA_HISTORY      // 0x0B — schema reply / Btm3 value push
        | can_class::BTM3_META_DATA           // 0x0C — Btm3 metadata reply
        | can_class::WRITE_ACK                // 0x10 — write/no-value ack
        | can_class::SCHEMA_DATA_NA           // 0x11 — short / "n/a" schema reply
        | 0x14                                // metadata error reply (e.g. EasyView's 0x0C errors)
    )
}
