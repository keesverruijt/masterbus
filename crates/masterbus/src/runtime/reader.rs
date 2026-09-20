//! Reader thread: drains the transport, routes discovery responses to the
//! waiter, caches values, and tracks device liveness.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::Sender;

use super::framelog::frame_log;
use super::state::State;
use super::waiter::Waiter;
use super::{Config, DeviceEvent};
use crate::model::{DeviceId, FieldId, field_id};
use crate::protocol::{
    BTM1_META_ADDR_FLAG, MbMessage, can_class, decode_value, frame_from_raw, parse_frame,
    waiter_key_for_frame,
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
        MbMessage::DeviceBroadcast {
            device_addr,
            type_code,
            firmware_version,
            ..
        } => {
            state.mark_alive(device_addr, type_code, firmware_version);
        }
        MbMessage::MonitoringData {
            device_addr,
            field_index,
            raw,
            ..
        } if device_addr & BTM1_META_ADDR_FLAG == 0 => {
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
        | 0x14 // metadata error reply (e.g. EasyView's 0x0C errors)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceStatus, FieldInfo, GroupInfo, Menu};
    use crate::protocol::VisualizationType;
    use crate::runtime::Config;
    use crate::runtime::fakebus::{ADDR, Device, FakeBus};
    use crate::value::Value;

    const ALIVE: Duration = Duration::from_secs(60);

    fn state_with_field(field: FieldId, viz: VisualizationType) -> State {
        let state = State::new();
        state.put_menu(
            ADDR,
            Menu::Monitoring,
            vec![GroupInfo {
                id: 0,
                name: "Monitoring".into(),
                menu: Menu::Monitoring,
                fields: vec![FieldInfo {
                    index: field,
                    name: "Voltage".into(),
                    unit: "V".into(),
                    viz_type: viz,
                    writeable: false,
                    eventable: false,
                    min: 0.0,
                    max: 0.0,
                    step: 0.0,
                    options: Vec::new(),
                }],
            }],
        );
        state
    }

    #[test]
    fn value_keys_are_channel_tagged() {
        assert_eq!(value_key(ADDR, field_id::btm1(0x17)), "val:188EA2:0017");
        assert_eq!(value_key(ADDR, field_id::btm3(0x17)), "val:188EA2:0117");
    }

    /// Only device-originated classes register a device. Master-side classes
    /// must not: SocketCAN loops our own requests back, and another master
    /// polling the bus would otherwise appear as a device.
    #[test]
    fn only_device_classes_register_a_device() {
        for class in [0x04, 0x06, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x10, 0x11, 0x14] {
            assert!(is_device_originated(class), "class 0x{class:02X}");
        }
        for class in [0x05, 0x07, 0x18, 0x19, 0x1A, 0x1B, 0x1C] {
            assert!(!is_device_originated(class), "class 0x{class:02X}");
        }
    }

    #[test]
    fn a_broadcast_registers_the_device() {
        let state = State::new();
        let waiter = Waiter::new();
        handle_frame(0x04_188EA2, &[0x0B, 0, 0, 0, 0x02, 0x01], &state, &waiter);

        assert_eq!(state.status(ADDR, ALIVE), DeviceStatus::On);
        assert_eq!(state.alive_ids(ALIVE), vec![ADDR]);
    }

    /// A truncated broadcast is not a device announcement — it parses as
    /// unknown and leaves the liveness table alone.
    #[test]
    fn a_short_broadcast_is_ignored() {
        let state = State::new();
        let waiter = Waiter::new();
        handle_frame(0x04_188EA2, &[0x0B, 0], &state, &waiter);

        // The class is still device-originated, so the device is *known*...
        assert_eq!(state.alive_ids(ALIVE), vec![ADDR]);
        // ...but nothing was decoded from the malformed payload.
        let map_is_empty = state.schema(ADDR).is_none() && state.identity(ADDR).is_none();
        assert!(map_is_empty);
    }

    /// Our own outbound request (class 0x18) looping back must not register a
    /// device — that's how the engine used to list itself and other masters.
    #[test]
    fn a_looped_back_request_registers_nothing() {
        let state = State::new();
        let waiter = Waiter::new();
        handle_frame(0x18_188EA2, &[0x17, 0x00], &state, &waiter);
        handle_frame(0x05_53A493, &[], &state, &waiter);

        assert!(!state.any_device());
    }

    /// A Btm1 value both wakes the pending read and lands in the cache,
    /// decoded per the field's visualization type.
    #[test]
    fn a_btm1_value_wakes_the_reader_and_is_cached() {
        let field = field_id::btm1(0x17);
        let state = state_with_field(field, VisualizationType::Float);
        let waiter = Waiter::new();
        waiter.register(&value_key(ADDR, field));

        let mut data = vec![0x17, 0x00];
        data.extend_from_slice(&12.5f32.to_le_bytes());
        handle_frame(0x08_188EA2, &data, &state, &waiter);

        assert_eq!(
            waiter.wait(&value_key(ADDR, field), Duration::ZERO),
            Some(12.5f32.to_le_bytes().to_vec())
        );
        assert_eq!(
            state.get_value(ADDR, field).unwrap().value,
            Value::Float(12.5)
        );
    }

    /// A value for a field we haven't discovered still wakes the waiter — the
    /// scheduler knows the type it asked for — but isn't cached blind.
    #[test]
    fn a_value_for_an_unknown_field_wakes_the_waiter_but_is_not_cached() {
        let state = State::new();
        let waiter = Waiter::new();
        let field = field_id::btm1(0x17);
        waiter.register(&value_key(ADDR, field));

        let mut data = vec![0x17, 0x00];
        data.extend_from_slice(&12.5f32.to_le_bytes());
        handle_frame(0x08_188EA2, &data, &state, &waiter);

        assert!(
            waiter
                .wait(&value_key(ADDR, field), Duration::ZERO)
                .is_some()
        );
        assert!(state.get_value(ADDR, field).is_none());
    }

    /// The Btm3 carrier: class 0x0B on the shadow address, headerless, and
    /// the same frame shape for an unsolicited push and a write-ack.
    #[test]
    fn a_btm3_push_on_the_shadow_address_is_cached_against_the_real_one() {
        let field = field_id::btm3(0x30);
        let state = State::new();
        state.put_all_fields(
            ADDR,
            vec![FieldInfo {
                index: field,
                name: "ShutDown".into(),
                unit: "V".into(),
                viz_type: VisualizationType::Float,
                writeable: true,
                eventable: false,
                min: 0.0,
                max: 0.0,
                step: 0.0,
                options: Vec::new(),
            }],
        );
        let waiter = Waiter::new();
        waiter.register(&value_key(ADDR, field));

        let mut data = vec![0x30, 0x00];
        data.extend_from_slice(&17.0f32.to_le_bytes());
        handle_frame(0x0B_988EA2, &data, &state, &waiter);

        assert!(
            waiter
                .wait(&value_key(ADDR, field), Duration::ZERO)
                .is_some()
        );
        assert_eq!(
            state.get_value(ADDR, field).unwrap().value,
            Value::Float(17.0)
        );
        // Registered against the real address, not the shadow one.
        assert_eq!(state.alive_ids(ALIVE), vec![ADDR]);
    }

    /// Metadata replies are routed to the discovery waiter by key, not to the
    /// value cache.
    #[test]
    fn a_metadata_reply_routes_to_its_discovery_key() {
        let state = State::new();
        let waiter = Waiter::new();
        let key = "btm1_meta:188EA2:02:23";
        waiter.register(key);

        handle_frame(
            0x08_988EA2,
            &[0x02, 0x17, 0x00, 0x00, 0x01],
            &state,
            &waiter,
        );

        assert_eq!(
            waiter.wait(key, Duration::ZERO),
            Some(vec![0x02, 0x17, 0x00, 0x00, 0x01])
        );
    }

    // ── liveness, over the fake bus ─────────────────────────────────────────

    fn test_config(liveness: Duration) -> Config {
        Config {
            min_send_interval: Duration::ZERO,
            discovery_window: Duration::ZERO,
            discovery_settle: Duration::ZERO,
            connect_timeout: Duration::from_secs(5),
            liveness,
            cache_path: None,
            ..Config::default()
        }
    }

    /// The reader diffs liveness each pass and emits presence events: a
    /// device that stops announcing goes offline, and comes back when it
    /// speaks again.
    #[test]
    fn a_device_that_stops_announcing_goes_offline_and_returns() {
        let (bus, transport) = FakeBus::start(Device::new());
        let engine =
            crate::runtime::Engine::connect(transport, test_config(Duration::from_millis(50)))
                .expect("connect");
        let events = engine.device_events();

        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Alive(ADDR))
        ));

        bus.with_device(|d| d.quiet = true);
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Offline(ADDR))
        ));
        assert!(engine.device_ids().is_empty());

        bus.with_device(|d| d.quiet = false);
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Alive(ADDR))
        ));
        assert_eq!(engine.device_ids(), vec![ADDR]);
    }
}
