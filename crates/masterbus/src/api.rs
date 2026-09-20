//! Public navigator API: `MasterBus` → `Device` → `Group` → `Field`, plus
//! rate-based subscriptions. Handles are cheap, `Clone`, `'static` (Arc-backed).

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;

use crate::error::{Error, Result};
use crate::model::{
    AccessLevel, DeviceId, DeviceIdentity, DeviceSchema, DeviceStatus, FieldId, FieldInfo,
    GroupInfo, Menu,
};
use crate::protocol::VisualizationType;
use crate::runtime::{Config, DeviceEvent, Engine, ValueUpdate};
use crate::settings::{DeviceType, FileConfig};
use crate::transport::Transport;
use crate::value::{Value, WriteValue};

/// A connection to a MasterBus.
#[derive(Clone)]
pub struct MasterBus {
    engine: Arc<Engine>,
}

impl MasterBus {
    /// Connect over a Linux SocketCAN interface.
    #[cfg(target_os = "linux")]
    pub fn socketcan(interface: &str, config: Config) -> Result<Self> {
        let t = crate::transport::socketcan::SocketCanTransport::open(interface)?;
        Self::with_transport(Box::new(t), config)
    }

    /// Connect over the Mastervolt USB link (cross-platform).
    pub fn usb(serial: Option<&str>, config: Config) -> Result<Self> {
        let t = crate::transport::usb::UsbTransport::open(serial)?;
        Self::with_transport(Box::new(t), config)
    }

    /// Connect over any [`Transport`].
    pub fn with_transport(transport: Box<dyn Transport>, config: Config) -> Result<Self> {
        Ok(MasterBus {
            engine: Engine::connect(transport, config)?,
        })
    }

    /// Connect using the standard per-host config file (see [`FileConfig`]).
    ///
    /// On first run the file is created with auto-detected values: a USB link
    /// if one is plugged in, otherwise the lone CAN interface. Path resolution
    /// and the creation step both log to stderr.
    ///
    /// `config` lets callers override any [`Config`] field; for
    /// [`Config::heartbeat_master`], a `Some` value wins, `None` falls back to
    /// the file's setting.
    pub fn auto(mut config: Config) -> Result<Self> {
        let file = FileConfig::load_or_create()?;
        if config.heartbeat_master.is_none() {
            config.heartbeat_master = file.heartbeat_master;
        }
        // Cache dir: explicit Config override wins; else the file's value, with
        // a fallback to $HOME/.cache/masterbus when the file's path isn't
        // writable by this user. `None` in the file means caching is off.
        if config.cache_path.is_none() {
            if let Some(requested) = &file.cache_dir {
                config.cache_path = crate::settings::resolve_cache_dir(requested);
            }
        }
        let name = if file.device_name.is_empty() {
            None
        } else {
            Some(file.device_name.as_str())
        };
        match file.device_type {
            DeviceType::Usb => Self::usb(name, config),
            #[cfg(target_os = "linux")]
            DeviceType::Can => {
                let iface = name.unwrap_or("can0");
                Self::socketcan(iface, config)
            }
            #[cfg(not(target_os = "linux"))]
            DeviceType::Can => Err(Error::Connection(format!(
                "{}: device_type = can but SocketCAN is Linux-only",
                file.path.display()
            ))),
        }
    }

    /// Currently-alive devices.
    pub fn devices(&self) -> Vec<Device> {
        self.engine
            .device_ids()
            .into_iter()
            .map(|id| Device {
                engine: self.engine.clone(),
                id,
            })
            .collect()
    }

    /// Like [`devices`](Self::devices) but first waits for the bus to fill
    /// in: at least [`Config::discovery_window`] after connect, and then
    /// until no new device has been heard for [`Config::discovery_settle`],
    /// so the whole bus is present. A device that stays silent longer than
    /// that (one that is powered up later, say) is only reported by
    /// [`device_events`](Self::device_events).
    pub fn devices_all(&self) -> Vec<Device> {
        self.engine
            .device_ids_all()
            .into_iter()
            .map(|id| Device {
                engine: self.engine.clone(),
                id,
            })
            .collect()
    }

    /// A handle to a specific device id (does not check presence).
    pub fn device(&self, id: DeviceId) -> Device {
        Device {
            engine: self.engine.clone(),
            id,
        }
    }

    /// Stream of device presence (alive/offline) events.
    pub fn device_events(&self) -> Receiver<DeviceEvent> {
        self.engine.device_events()
    }

    /// Subscribe to live updates of `fields` on `device` at `interval`.
    pub fn subscribe(
        &self,
        device: DeviceId,
        fields: impl IntoIterator<Item = FieldId>,
        interval: Duration,
        change_only: bool,
    ) -> Subscription {
        let (id, rx) =
            self.engine
                .subscribe(device, fields.into_iter().collect(), interval, change_only);
        Subscription {
            engine: self.engine.clone(),
            id,
            rx,
        }
    }
}

/// A device on the bus.
#[derive(Clone)]
pub struct Device {
    engine: Arc<Engine>,
    id: DeviceId,
}

impl Device {
    /// The device id (its 24-bit CAN address).
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Fetch (discovering if needed) and clone the full schema.
    pub fn schema(&self) -> Result<DeviceSchema> {
        self.engine.ensure_schema(self.id)?;
        self.engine.state.schema(self.id).ok_or(Error::NotReady)
    }

    /// Fetch (cheaply — identity only, no group enumeration) the device identity.
    pub fn identity(&self) -> Result<DeviceIdentity> {
        self.engine.identity(self.id)
    }

    /// Article number (identity-only discovery).
    pub fn article_number(&self) -> Result<String> {
        Ok(self.identity()?.article)
    }
    /// Serial number (identity-only discovery).
    pub fn serial_number(&self) -> Result<String> {
        Ok(self.identity()?.serial)
    }
    /// Revision code (identity-only discovery).
    pub fn revision_code(&self) -> Result<String> {
        Ok(self.identity()?.revision)
    }
    /// Human-readable name (identity-only discovery).
    pub fn name(&self) -> Result<String> {
        Ok(self.identity()?.name)
    }
    /// Firmware version (identity-only discovery).
    pub fn firmware_version(&self) -> Result<String> {
        Ok(self.identity()?.firmware)
    }

    /// Liveness-derived status.
    pub fn status(&self) -> DeviceStatus {
        self.engine
            .state
            .status(self.id, self.engine.config.liveness)
    }

    /// All groups (across menus).
    pub fn groups(&self) -> Result<Vec<Group>> {
        let schema = self.schema()?;
        Ok(schema
            .groups
            .iter()
            .map(|g| Group {
                engine: self.engine.clone(),
                device: self.id,
                group_id: g.id,
            })
            .collect())
    }

    /// Groups belonging to a particular menu / access level. Only that menu is
    /// discovered (lazily), not the whole device.
    pub fn tab(&self, menu: Menu) -> Result<Vec<Group>> {
        self.engine.ensure_menu(self.id, menu)?;
        let schema = self.engine.state.schema(self.id).ok_or(Error::NotReady)?;
        Ok(schema
            .groups
            .iter()
            .filter(|g| g.menu == menu)
            .map(|g| Group {
                engine: self.engine.clone(),
                device: self.id,
                group_id: g.id,
            })
            .collect())
    }

    /// Discover (only) `menu` and return its groups as raw [`GroupInfo`]
    /// (name, fields with metadata). Convenient for building a UI tab without
    /// triggering full-device discovery.
    pub fn tab_info(&self, menu: Menu) -> Result<Vec<GroupInfo>> {
        self.engine.ensure_menu(self.id, menu)?;
        let schema = self.engine.state.schema(self.id).ok_or(Error::NotReady)?;
        Ok(schema
            .groups
            .into_iter()
            .filter(|g| g.menu == menu)
            .collect())
    }

    /// A handle to a field by its channel-aware id.
    pub fn field(&self, index: FieldId) -> Field {
        Field {
            engine: self.engine.clone(),
            device: self.id,
            index,
        }
    }

    /// Flat probe of the device's entire field-index space, ignoring the
    /// per-menu group counts (which lie on some devices — see PROTOCOL.md
    /// §4.3 + FINDINGS for the Magic-class Nav Chg). Returns every field
    /// index that responded to a metadata query, with no group structure.
    pub fn all_fields(&self) -> Result<Vec<FieldInfo>> {
        self.engine.ensure_all_fields(self.id)?;
        self.engine.state.all_fields(self.id).ok_or(Error::NotReady)
    }

    /// Names of this device's **eventable outputs**, in field-index order — the
    /// target space an `Event N command` on another device selects into. An
    /// event's command index `K` maps to `eventable_outputs()[K]` on the
    /// event's target device.
    ///
    /// Reads only **already-discovered** fields (schema groups + the flat
    /// probe); it does not trigger discovery, so it is empty until the device's
    /// configuration has been enumerated. See PROTOCOL.md §9a.
    pub fn eventable_outputs(&self) -> Vec<String> {
        let mut evt: Vec<(FieldId, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut add = |f: &FieldInfo| {
            if f.eventable && seen.insert(f.index) {
                evt.push((f.index, f.name.clone()));
            }
        };
        if let Some(schema) = self.engine.state.schema(self.id) {
            for g in &schema.groups {
                for f in &g.fields {
                    add(f);
                }
            }
        }
        if let Some(all) = self.engine.state.all_fields(self.id) {
            for f in &all {
                add(f);
            }
        }
        evt.sort_by_key(|(i, _)| *i);
        evt.into_iter().map(|(_, n)| n).collect()
    }

    /// Read the device's current access level (PROTOCOL.md §4.5).
    ///
    /// Note: a level change does not always change the index space, but it
    /// does change which fields are writable. After a successful
    /// [`Self::login`] or [`Self::logout`], any cached `is_writable` results
    /// for this device's fields should be re-queried.
    pub fn access_level(&self) -> Result<AccessLevel> {
        self.engine.access_level(self.id)
    }

    /// Cached access level, if known — does **not** issue a wire query.
    /// `None` until the level has been observed at least once (either via
    /// [`Self::access_level`], [`Self::login`], [`Self::logout`], or any
    /// scheduler-side discovery that needed to record it). Cheap enough to
    /// call from a render loop.
    pub fn cached_access_level(&self) -> Option<AccessLevel> {
        self.engine.state.access_level(self.id)
    }

    /// Attempt to log this device in at `level` using the f32 access `code`.
    /// The code is vendor-defined per device family; this crate is opaque
    /// to its value. The caller supplies whatever bytes they want sent.
    ///
    /// Returns the level the device reports *after* the request. If the
    /// code was wrong the device silently keeps the previous level — the
    /// returned `AccessLevel` will not equal `level`. Compare the result
    /// to the prior level to distinguish success from a rejected code.
    pub fn login(&self, level: AccessLevel, code: f32) -> Result<AccessLevel> {
        self.engine.set_access_level(self.id, level, Some(code))
    }

    /// Log out of the device (return to `AccessLevel::EndUser`). Wire form is
    /// a 4-byte request with no code payload (PROTOCOL.md §4.5).
    pub fn logout(&self) -> Result<AccessLevel> {
        self.engine
            .set_access_level(self.id, AccessLevel::EndUser, None)
    }

    /// Write the device's editable string table at `str_id` (PROTOCOL.md
    /// §4.4 write direction). Used for editable Text-viz fields such as
    /// "Device name". The string id is supplied explicitly: a Text field's
    /// writable sid isn't yet derivable from metadata, so the caller picks
    /// the slot from out-of-band knowledge (e.g. captured traffic) until
    /// the discovery path is wired up.
    pub fn write_string(&self, str_id: u16, text: &str) -> Result<()> {
        self.engine.write_string(self.id, str_id, text)
    }
}

/// A group of fields within a device.
#[derive(Clone)]
pub struct Group {
    engine: Arc<Engine>,
    device: DeviceId,
    group_id: i32,
}

impl Group {
    fn info(&self) -> Result<GroupInfo> {
        let find = |s: DeviceSchema| s.groups.into_iter().find(|g| g.id == self.group_id);
        // The group usually came from `tab()`/`groups()`, so it's already known.
        if let Some(g) = self.engine.state.schema(self.device).and_then(find) {
            return Ok(g);
        }
        self.engine.ensure_schema(self.device)?;
        self.engine
            .state
            .schema(self.device)
            .and_then(find)
            .ok_or(Error::GroupNotAvailable(self.group_id))
    }

    /// Group name.
    pub fn name(&self) -> Result<String> {
        Ok(self.info()?.name)
    }

    /// Which menu / access level this group belongs to.
    pub fn menu(&self) -> Result<Menu> {
        Ok(self.info()?.menu)
    }

    /// Fields in this group.
    pub fn fields(&self) -> Result<Vec<Field>> {
        Ok(self
            .info()?
            .fields
            .iter()
            .map(|f| Field {
                engine: self.engine.clone(),
                device: self.device,
                index: f.index,
            })
            .collect())
    }
}

/// A single field of a device.
#[derive(Clone)]
pub struct Field {
    engine: Arc<Engine>,
    device: DeviceId,
    index: FieldId,
}

impl Field {
    /// The field's full schema entry (name, unit, viz type, writeable flag,
    /// numeric bounds, and list/enum option labels). Pairs with [`value`](Self::value)
    /// so a caller can resolve a list value's index to its label.
    pub fn info(&self) -> Result<FieldInfo> {
        self.engine.ensure_field(self.device, self.index)?;
        self.engine
            .state
            .field_info(self.device, self.index)
            .ok_or(Error::FieldNotAvailable(self.index as i32))
    }

    /// Channel-aware field id (channel in bit 8, wire index in bits 0..8).
    pub fn index(&self) -> FieldId {
        self.index
    }
    /// Field name.
    pub fn name(&self) -> Result<String> {
        Ok(self.info()?.name)
    }
    /// Unit (may be empty).
    pub fn unit(&self) -> Result<String> {
        Ok(self.info()?.unit)
    }
    /// Visualization / value type.
    pub fn viz_type(&self) -> Result<VisualizationType> {
        Ok(self.info()?.viz_type)
    }
    /// Whether the field is currently writable.
    pub fn is_writable(&self) -> Result<bool> {
        Ok(self.info()?.writeable)
    }

    /// Read the current value (cache if fresh, else poll).
    pub fn value(&self) -> Result<Value> {
        self.engine
            .read(self.device, self.index, self.engine.config.max_age)
    }

    /// Write a value; returns the value observed after the write.
    ///
    /// The supplied [`Value`] must match the field's schema type (e.g. a
    /// `Boolean` for a checkbox field, a `Float` for a numeric field), otherwise
    /// [`Error::WrongType`] is returned and nothing is sent.
    pub fn set(&self, value: Value) -> Result<Value> {
        let info = self.info()?;
        if !info.writeable {
            return Err(Error::ReadOnly);
        }
        let wv = write_value_for(info.viz_type, value)?;
        if let WriteValue::Text { text, .. } = &wv {
            validate_editable_text(text)?;
        }
        self.engine.write(self.device, self.index, wv)
    }

    /// Subscribe to live updates of just this field.
    pub fn subscribe(&self, interval: Duration, change_only: bool) -> Subscription {
        let (id, rx) = self
            .engine
            .subscribe(self.device, vec![self.index], interval, change_only);
        Subscription {
            engine: self.engine.clone(),
            id,
            rx,
        }
    }
}

/// Maximum byte length of an editable MasterBus string. The wire protocol
/// (PROTOCOL.md §4.4) chunks strings in 4-byte groups and NUL-terminates them;
/// every observed device caps editable slots at 16 characters of printable
/// ASCII (the limit MasterAdjust enforces on its input widgets).
pub const MAX_EDITABLE_TEXT_BYTES: usize = 16;

/// Validate a candidate string-write payload against the wire constraints.
fn validate_editable_text(s: &str) -> Result<()> {
    if s.len() > MAX_EDITABLE_TEXT_BYTES {
        return Err(Error::InvalidText {
            got: s.len(),
            issue: format!("too long (max {MAX_EDITABLE_TEXT_BYTES})"),
        });
    }
    for (i, b) in s.bytes().enumerate() {
        if b == 0 {
            return Err(Error::InvalidText {
                got: s.len(),
                issue: format!("embedded NUL at byte {i}"),
            });
        }
        if !(0x20..=0x7E).contains(&b) {
            return Err(Error::InvalidText {
                got: s.len(),
                issue: format!("non-printable byte 0x{b:02X} at byte {i}"),
            });
        }
    }
    Ok(())
}

/// Validate a [`Value`] against the field's [`VisualizationType`] and convert it
/// to the wire [`WriteValue`]. Returns [`Error::WrongType`] on a type mismatch.
fn write_value_for(viz: VisualizationType, value: Value) -> Result<WriteValue> {
    use VisualizationType as V;
    let wrong = |expected| Err(Error::WrongType { expected });
    match viz {
        V::Float => match value {
            Value::Float(f) => Ok(WriteValue::Float(f)),
            _ => wrong("Float"),
        },
        V::CheckBox | V::ToggleButton | V::PushButton => match value {
            Value::Boolean(b) => Ok(WriteValue::Bool(b)),
            _ => wrong("Boolean"),
        },
        // Radio/DropDown and the event-command selector are all a list index.
        V::Radio | V::DropDown | V::EventCommand => match value {
            Value::List { index, .. } => Ok(WriteValue::ListIndex(index)),
            _ => wrong("List"),
        },
        V::Eventable => match value {
            Value::Eventable { index, .. } => Ok(WriteValue::ListIndex(index)),
            _ => wrong("Eventable"),
        },
        V::DeviceList => match value {
            Value::DeviceRef { index, .. } => Ok(WriteValue::ListIndex(index)),
            _ => wrong("DeviceRef"),
        },
        V::Text => match value {
            Value::Text { sid, text } => Ok(WriteValue::Text { sid, text }),
            _ => wrong("Text"),
        },
        // Greyed (read-only display) and Date/Time have no write encoding.
        V::GrayVisualization | V::Date | V::Time => wrong("a writable type"),
    }
}

/// A live subscription; unsubscribes on drop.
pub struct Subscription {
    engine: Arc<Engine>,
    id: u64,
    rx: Receiver<ValueUpdate>,
}

impl Subscription {
    /// The update receiver (use directly or via `select!`).
    pub fn receiver(&self) -> &Receiver<ValueUpdate> {
        &self.rx
    }
    /// Block for the next update.
    pub fn recv(&self) -> Option<ValueUpdate> {
        self.rx.recv().ok()
    }
    /// Non-blocking poll for an update.
    pub fn try_recv(&self) -> Option<ValueUpdate> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.engine.unsubscribe(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::field_id;
    use crate::runtime::fakebus::{ADDR, Device as FakeDevice, FakeBus};
    use crate::value::Date;
    use std::time::Instant;

    /// Every menu, so seeding them all marks the device fully discovered and
    /// the handle layer runs without a discovery round trip.
    const ALL_MENUS: [Menu; 5] = [
        Menu::Monitoring,
        Menu::Configuration,
        Menu::Service,
        Menu::Alarm,
        Menu::History,
    ];

    fn test_config() -> Config {
        Config {
            min_send_interval: Duration::ZERO,
            discovery_timeout: Duration::from_millis(5),
            discovery_retries: 1,
            discovery_window: Duration::ZERO,
            discovery_settle: Duration::ZERO,
            connect_timeout: Duration::from_secs(5),
            liveness: Duration::from_millis(200),
            cache_path: None,
            ..Config::default()
        }
    }

    fn connect(device: FakeDevice, config: Config) -> (MasterBus, FakeBus) {
        let (fake, transport) = FakeBus::start(device);
        let bus = MasterBus::with_transport(transport, config).expect("connect");
        (bus, fake)
    }

    fn field_info(
        index: FieldId,
        name: &str,
        viz: VisualizationType,
        writeable: bool,
    ) -> FieldInfo {
        FieldInfo {
            index,
            name: name.to_string(),
            unit: "V".into(),
            viz_type: viz,
            writeable,
            eventable: false,
            min: 0.0,
            max: 0.0,
            step: 0.0,
            options: Vec::new(),
        }
    }

    fn group_info(id: i32, name: &str, menu: Menu, fields: Vec<FieldInfo>) -> GroupInfo {
        GroupInfo {
            id,
            name: name.to_string(),
            menu,
            fields,
        }
    }

    /// Put a fully-discovered device in place: identity plus every menu, so
    /// each `ensure_*` call is satisfied from memory.
    fn seed(bus: &MasterBus, groups: Vec<GroupInfo>) {
        let state = &bus.engine.state;
        state.put_identity(
            ADDR,
            DeviceIdentity {
                article: "44010250".into(),
                serial: "1234567".into(),
                revision: "3".into(),
                name: "Combi".into(),
                firmware: "1.0".into(),
            },
        );
        for menu in ALL_MENUS {
            let of_menu: Vec<GroupInfo> =
                groups.iter().filter(|g| g.menu == menu).cloned().collect();
            state.put_menu(ADDR, menu, of_menu);
        }
    }

    /// One writable Float, one read-only Float, one Text — enough to exercise
    /// every `Field` accessor and both `set` rejections.
    fn seed_default(bus: &MasterBus) {
        seed(
            bus,
            vec![
                group_info(
                    0,
                    "DC",
                    Menu::Monitoring,
                    vec![
                        field_info(
                            field_id::btm1(0x17),
                            "Voltage",
                            VisualizationType::Float,
                            true,
                        ),
                        field_info(
                            field_id::btm1(0x18),
                            "Current",
                            VisualizationType::Float,
                            false,
                        ),
                    ],
                ),
                group_info(
                    7,
                    "General",
                    Menu::Configuration,
                    vec![field_info(
                        field_id::btm1(0x01),
                        "Device name",
                        VisualizationType::Text,
                        true,
                    )],
                ),
            ],
        );
    }

    // ── MasterBus ───────────────────────────────────────────────────────────

    #[test]
    fn the_bus_lists_its_live_devices() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());

        let devices = bus.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id(), ADDR);
        assert_eq!(devices[0].status(), DeviceStatus::On);
    }

    /// A handle by id is just a handle: no presence check, no traffic.
    #[test]
    fn a_handle_by_id_does_not_check_presence() {
        let (bus, fake) = connect(FakeDevice::new(), test_config());

        let absent = bus.device(0x00DEAD);
        assert_eq!(absent.id(), 0x00DEAD);
        assert_eq!(absent.status(), DeviceStatus::Offline);
        assert!(fake.sent().is_empty());
    }

    /// `devices_all` holds off until the collection window has passed, so a
    /// caller enumerating the bus sees all of it rather than whoever spoke
    /// first.
    #[test]
    fn devices_all_waits_for_the_collection_window() {
        let config = Config {
            discovery_window: Duration::from_millis(150),
            ..test_config()
        };
        let started = Instant::now();
        let (bus, _fake) = connect(FakeDevice::new(), config);

        let devices = bus.devices_all();
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id(), ADDR);
    }

    #[test]
    fn the_event_stream_reports_the_first_device() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());

        assert!(matches!(
            bus.device_events().recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Alive(ADDR))
        ));
    }

    // ── Device ──────────────────────────────────────────────────────────────

    /// Every identity accessor is a view on one cheap fetch.
    #[test]
    fn the_identity_accessors_share_one_fetch() {
        let device = FakeDevice::new().with_identity("44010250", "1234567", "Combi");
        let (bus, fake) = connect(device, test_config());
        let d = bus.device(ADDR);

        assert_eq!(d.article_number().unwrap(), "44010250");
        assert_eq!(d.serial_number().unwrap(), "1234567");
        assert_eq!(d.name().unwrap(), "Combi");
        assert_eq!(d.revision_code().unwrap(), "3");
        assert_eq!(d.firmware_version().unwrap(), "1.0");
        assert_eq!(d.identity().unwrap().article, "44010250");

        // Only the first accessor went to the wire.
        let queries = fake.sent().len();
        assert_eq!(d.name().unwrap(), "Combi");
        assert_eq!(fake.sent().len(), queries);
    }

    #[test]
    fn the_schema_is_exposed_as_groups_and_tabs() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        seed_default(&bus);
        let d = bus.device(ADDR);

        assert_eq!(d.schema().unwrap().groups.len(), 2);
        assert_eq!(d.groups().unwrap().len(), 2);

        let monitoring = d.tab(Menu::Monitoring).unwrap();
        assert_eq!(monitoring.len(), 1);
        assert_eq!(monitoring[0].name().unwrap(), "DC");

        let info = d.tab_info(Menu::Configuration).unwrap();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].fields[0].name, "Device name");

        assert!(d.tab(Menu::Service).unwrap().is_empty());
    }

    #[test]
    fn the_flat_probe_list_is_exposed() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        bus.engine.state.put_all_fields(
            ADDR,
            vec![field_info(
                field_id::btm3(0x30),
                "ShutDown",
                VisualizationType::Float,
                true,
            )],
        );

        let all = bus.device(ADDR).all_fields().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "ShutDown");
    }

    /// The event-target space: eventable fields from both the schema and the
    /// flat probe, in field-index order, each named once.
    #[test]
    fn eventable_outputs_are_index_ordered_and_deduplicated() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        let eventable = |index, name| FieldInfo {
            eventable: true,
            ..field_info(index, name, VisualizationType::Eventable, true)
        };
        seed(
            &bus,
            vec![group_info(
                0,
                "Relays",
                Menu::Monitoring,
                vec![
                    eventable(field_id::btm1(0x20), "Relay 2"),
                    field_info(
                        field_id::btm1(0x05),
                        "Voltage",
                        VisualizationType::Float,
                        false,
                    ),
                    eventable(field_id::btm1(0x10), "Relay 1"),
                ],
            )],
        );
        // The flat probe repeats one of them and adds a third.
        bus.engine.state.put_all_fields(
            ADDR,
            vec![
                eventable(field_id::btm1(0x10), "Relay 1"),
                eventable(field_id::btm1(0x30), "Relay 3"),
            ],
        );

        assert_eq!(
            bus.device(ADDR).eventable_outputs(),
            vec!["Relay 1", "Relay 2", "Relay 3"]
        );
    }

    /// Nothing is known about the level until it has been observed; asking
    /// costs a round trip, and the answer is cached for the render loop.
    #[test]
    fn the_access_level_is_cached_only_once_observed() {
        let mut device = FakeDevice::new();
        device.level = AccessLevel::Installer;
        let (bus, _fake) = connect(device, test_config());
        let d = bus.device(ADDR);

        assert!(d.cached_access_level().is_none());
        assert_eq!(d.access_level().unwrap(), AccessLevel::Installer);
        assert_eq!(d.cached_access_level(), Some(AccessLevel::Installer));
    }

    #[test]
    fn login_and_logout_go_through_the_handle() {
        let mut device = FakeDevice::new();
        device.codes.insert(1, 1234.0);
        let (bus, fake) = connect(device, test_config());
        let d = bus.device(ADDR);

        assert_eq!(
            d.login(AccessLevel::Installer, 1234.0).unwrap(),
            AccessLevel::Installer
        );
        assert_eq!(d.logout().unwrap(), AccessLevel::EndUser);
        assert_eq!(fake.device.lock().unwrap().level, AccessLevel::EndUser);
    }

    #[test]
    fn a_string_write_goes_through_the_handle() {
        let (bus, fake) = connect(FakeDevice::new(), test_config());

        bus.device(ADDR).write_string(0x0001, "Combi").unwrap();
        assert_eq!(fake.device.lock().unwrap().strings[&0x0001], "Combi");
    }

    // ── Group ───────────────────────────────────────────────────────────────

    #[test]
    fn a_group_exposes_its_name_menu_and_fields() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        seed_default(&bus);

        let group = &bus.device(ADDR).tab(Menu::Monitoring).unwrap()[0];
        assert_eq!(group.name().unwrap(), "DC");
        assert_eq!(group.menu().unwrap(), Menu::Monitoring);

        let fields = group.fields().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name().unwrap(), "Voltage");
    }

    /// A group handle that outlives its schema (or was never in one) reports
    /// the group as unavailable rather than panicking.
    #[test]
    fn an_unknown_group_is_reported_unavailable() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        seed_default(&bus);

        let ghost = Group {
            engine: bus.engine.clone(),
            device: ADDR,
            group_id: 99,
        };
        assert!(matches!(ghost.name(), Err(Error::GroupNotAvailable(99))));
    }

    // ── Field ───────────────────────────────────────────────────────────────

    #[test]
    fn the_field_accessors_read_the_schema_entry() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        seed_default(&bus);

        let f = bus.device(ADDR).field(field_id::btm1(0x17));
        assert_eq!(f.index(), field_id::btm1(0x17));
        assert_eq!(f.name().unwrap(), "Voltage");
        assert_eq!(f.unit().unwrap(), "V");
        assert_eq!(f.viz_type().unwrap(), VisualizationType::Float);
        assert!(f.is_writable().unwrap());
        assert!(
            !bus.device(ADDR)
                .field(field_id::btm1(0x18))
                .is_writable()
                .unwrap()
        );
    }

    #[test]
    fn a_field_on_a_fully_discovered_device_that_does_not_exist_is_unavailable() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        seed_default(&bus);

        assert!(matches!(
            bus.device(ADDR).field(field_id::btm1(0x99)).info(),
            Err(Error::FieldNotAvailable(0x99))
        ));
    }

    #[test]
    fn a_field_reads_its_value_from_the_device() {
        let device = FakeDevice::new().with_btm1(0x17, 12.5);
        let (bus, _fake) = connect(device, test_config());
        seed_default(&bus);

        assert_eq!(
            bus.device(ADDR)
                .field(field_id::btm1(0x17))
                .value()
                .unwrap(),
            Value::Float(12.5)
        );
    }

    #[test]
    fn a_write_lands_and_returns_the_observed_value() {
        let device = FakeDevice::new().with_btm1(0x17, 12.5);
        let (bus, fake) = connect(device, test_config());
        seed_default(&bus);

        let observed = bus
            .device(ADDR)
            .field(field_id::btm1(0x17))
            .set(Value::Float(13.2))
            .unwrap();
        assert_eq!(observed, Value::Float(13.2));
        assert_eq!(
            fake.device.lock().unwrap().btm1[&0x17],
            13.2f32.to_le_bytes()
        );
    }

    /// The three ways `set` refuses before anything reaches the bus: a value
    /// of the wrong kind, a read-only field, and text the wire cannot carry.
    #[test]
    fn a_rejected_write_never_reaches_the_bus() {
        let (bus, fake) = connect(FakeDevice::new(), test_config());
        seed_default(&bus);
        let d = bus.device(ADDR);

        assert!(matches!(
            d.field(field_id::btm1(0x17)).set(Value::Boolean(true)),
            Err(Error::WrongType { expected: "Float" })
        ));
        assert!(matches!(
            d.field(field_id::btm1(0x18)).set(Value::Float(1.0)),
            Err(Error::ReadOnly)
        ));
        assert!(matches!(
            d.field(field_id::btm1(0x01)).set(Value::Text {
                sid: 1,
                text: "far too long to fit".into()
            }),
            Err(Error::InvalidText { got: 19, .. })
        ));

        assert!(fake.sent().is_empty());
    }

    // ── Subscription ────────────────────────────────────────────────────────

    /// A field subscription delivers updates and cancels itself on drop.
    #[test]
    fn a_subscription_delivers_and_cancels_on_drop() {
        let device = FakeDevice::new().with_btm1(0x17, 12.5);
        let (bus, _fake) = connect(device, test_config());
        seed_default(&bus);

        let sub = bus
            .device(ADDR)
            .field(field_id::btm1(0x17))
            .subscribe(Duration::from_millis(10), false);

        let update = sub.recv().unwrap();
        assert_eq!(update.device, ADDR);
        assert_eq!(update.field, field_id::btm1(0x17));
        assert_eq!(update.value, Value::Float(12.5));
        assert!(sub.receiver().recv_timeout(Duration::from_secs(2)).is_ok());

        let rx = sub.receiver().clone();
        drop(sub);
        // Drain what was already in flight, then expect silence.
        while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    /// The bus-level subscription takes several fields at once.
    #[test]
    fn a_bus_subscription_covers_several_fields() {
        let device = FakeDevice::new().with_btm1(0x17, 12.5).with_btm1(0x18, 3.0);
        let (bus, _fake) = connect(device, test_config());
        seed_default(&bus);

        let sub = bus.subscribe(
            ADDR,
            [field_id::btm1(0x17), field_id::btm1(0x18)],
            Duration::from_millis(10),
            false,
        );

        let mut seen = std::collections::HashSet::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.len() < 2 && Instant::now() < deadline {
            if let Some(u) = sub.try_recv() {
                seen.insert(u.field);
            }
        }
        assert_eq!(seen.len(), 2, "expected both fields, saw {seen:?}");
    }

    /// `eventable_outputs` reads only what discovery has already found, so an
    /// un-enumerated device has no target space rather than an error.
    #[test]
    fn eventable_outputs_are_empty_before_discovery() {
        let (bus, _fake) = connect(FakeDevice::new(), test_config());
        assert!(bus.device(ADDR).eventable_outputs().is_empty());
    }

    /// The full write-type gate, one row per visualization type: what a field
    /// of that type accepts, and that anything else is refused before a byte
    /// goes out. The read-only display types accept nothing at all.
    #[test]
    fn every_viz_type_has_a_write_rule() {
        use VisualizationType as V;
        let list = || Value::List {
            index: 2,
            options: vec![],
        };
        let eventable = || Value::Eventable {
            index: 2,
            labels: vec![],
        };
        let device_ref = || Value::DeviceRef {
            index: 2,
            device_ids: vec![],
        };
        let text = || Value::Text {
            sid: 1,
            text: "Combi".into(),
        };

        let accepted: Vec<(V, Value, WriteValue)> = vec![
            (V::Float, Value::Float(4.0), WriteValue::Float(4.0)),
            (V::CheckBox, Value::Boolean(true), WriteValue::Bool(true)),
            (
                V::ToggleButton,
                Value::Boolean(false),
                WriteValue::Bool(false),
            ),
            (V::PushButton, Value::Boolean(true), WriteValue::Bool(true)),
            (V::Radio, list(), WriteValue::ListIndex(2)),
            (V::DropDown, list(), WriteValue::ListIndex(2)),
            (V::EventCommand, list(), WriteValue::ListIndex(2)),
            (V::Eventable, eventable(), WriteValue::ListIndex(2)),
            (V::DeviceList, device_ref(), WriteValue::ListIndex(2)),
            (
                V::Text,
                text(),
                WriteValue::Text {
                    sid: 1,
                    text: "Combi".into(),
                },
            ),
        ];
        for (viz, value, want) in accepted {
            assert_eq!(
                write_value_for(viz, value).unwrap(),
                want,
                "{viz:?} should accept its own value type"
            );
        }

        // Every writable type refuses a value of the wrong kind...
        for viz in [
            V::Float,
            V::CheckBox,
            V::ToggleButton,
            V::PushButton,
            V::Radio,
            V::DropDown,
            V::EventCommand,
            V::Eventable,
            V::DeviceList,
            V::Text,
        ] {
            let wrong = if matches!(viz, V::Float) {
                Value::Boolean(true)
            } else {
                Value::Float(1.0)
            };
            assert!(
                matches!(write_value_for(viz, wrong), Err(Error::WrongType { .. })),
                "{viz:?} should reject a mismatched value"
            );
        }

        // ...and the display-only types have no write encoding at all.
        for viz in [V::GrayVisualization, V::Time, V::Date] {
            assert!(
                matches!(
                    write_value_for(viz, Value::Float(1.0)),
                    Err(Error::WrongType {
                        expected: "a writable type"
                    })
                ),
                "{viz:?} should not be writable"
            );
        }
    }

    #[test]
    fn write_value_matches_schema_type() {
        // Right types map through.
        assert!(matches!(
            write_value_for(VisualizationType::Float, Value::Float(4.0)),
            Ok(WriteValue::Float(_))
        ));
        assert!(matches!(
            write_value_for(VisualizationType::CheckBox, Value::Boolean(true)),
            Ok(WriteValue::Bool(true))
        ));
        assert!(matches!(
            write_value_for(
                VisualizationType::DropDown,
                Value::List {
                    index: 2,
                    options: vec![]
                }
            ),
            Ok(WriteValue::ListIndex(2))
        ));
    }

    #[test]
    fn write_value_rejects_mismatch() {
        // Boolean into a numeric field, etc.
        assert!(matches!(
            write_value_for(VisualizationType::Float, Value::Boolean(true)),
            Err(Error::WrongType { .. })
        ));
        assert!(matches!(
            write_value_for(VisualizationType::CheckBox, Value::Float(1.0)),
            Err(Error::WrongType { .. })
        ));
        // Editable-text validator: 16-byte printable-ASCII cap, no NULs.
        assert!(validate_editable_text("Nav Chg").is_ok());
        assert!(validate_editable_text("01234567890123456").is_err()); // 17 chars
        assert!(validate_editable_text("hi\0there").is_err()); // embedded NUL
        assert!(validate_editable_text("hé").is_err()); // non-ASCII
        // Non-writable field types are rejected.
        assert!(matches!(
            write_value_for(
                VisualizationType::Date,
                Value::Date(Date {
                    day: 1,
                    mon: 1,
                    year: 2026
                })
            ),
            Err(Error::WrongType { .. })
        ));
    }
}
