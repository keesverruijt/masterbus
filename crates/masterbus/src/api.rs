//! Public navigator API: `MasterBus` → `Device` → `Group` → `Field`, plus
//! rate-based subscriptions. Handles are cheap, `Clone`, `'static` (Arc-backed).

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;

use crate::error::{Error, Result};
use crate::model::{DeviceSchema, DeviceStatus, FieldInfo, GroupInfo, Menu};
use crate::runtime::{Config, DeviceEvent, Engine, ValueUpdate};
use crate::protocol::VisualizationType;
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
    #[cfg(feature = "usb")]
    pub fn usb(serial: Option<&str>, config: Config) -> Result<Self> {
        let t = crate::transport::usb::UsbTransport::open(serial)?;
        Self::with_transport(Box::new(t), config)
    }

    /// Connect over any [`Transport`].
    pub fn with_transport(transport: Box<dyn Transport>, config: Config) -> Result<Self> {
        Ok(MasterBus { engine: Engine::connect(transport, config)? })
    }

    /// Currently-alive devices.
    pub fn devices(&self) -> Vec<Device> {
        self.engine
            .device_ids()
            .into_iter()
            .map(|id| Device { engine: self.engine.clone(), id })
            .collect()
    }

    /// Like [`devices`](Self::devices) but first waits until the broadcast
    /// window (2 s after connect) has elapsed, so the whole bus is present.
    pub fn devices_all(&self) -> Vec<Device> {
        self.engine
            .device_ids_all()
            .into_iter()
            .map(|id| Device { engine: self.engine.clone(), id })
            .collect()
    }

    /// A handle to a specific device id (does not check presence).
    pub fn device(&self, id: u32) -> Device {
        Device { engine: self.engine.clone(), id }
    }

    /// Stream of device presence (alive/offline) events.
    pub fn device_events(&self) -> Receiver<DeviceEvent> {
        self.engine.device_events()
    }

    /// Subscribe to live updates of `fields` on `device` at `interval`.
    pub fn subscribe(
        &self,
        device: u32,
        fields: impl IntoIterator<Item = i32>,
        interval: Duration,
        change_only: bool,
    ) -> Subscription {
        let (id, rx) =
            self.engine.subscribe(device, fields.into_iter().collect(), interval, change_only);
        Subscription { engine: self.engine.clone(), id, rx }
    }
}

/// A device on the bus.
#[derive(Clone)]
pub struct Device {
    engine: Arc<Engine>,
    id: u32,
}

impl Device {
    /// The device id (its CAN address).
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Fetch (discovering if needed) and clone the full schema.
    pub fn schema(&self) -> Result<DeviceSchema> {
        self.engine.ensure_schema(self.id)?;
        self.engine.state.schema(self.id).ok_or(Error::NotReady)
    }

    /// Article number.
    pub fn article_number(&self) -> Result<String> {
        Ok(self.schema()?.article)
    }
    /// Serial number.
    pub fn serial_number(&self) -> Result<String> {
        Ok(self.schema()?.serial)
    }
    /// Revision code.
    pub fn revision_code(&self) -> Result<String> {
        Ok(self.schema()?.revision)
    }
    /// Human-readable name.
    pub fn name(&self) -> Result<String> {
        Ok(self.schema()?.name)
    }
    /// Firmware version.
    pub fn firmware_version(&self) -> Result<String> {
        Ok(self.schema()?.firmware)
    }

    /// Liveness-derived status.
    pub fn status(&self) -> DeviceStatus {
        self.engine.state.status(self.id, self.engine.config.liveness)
    }

    /// All groups (across menus).
    pub fn groups(&self) -> Result<Vec<Group>> {
        let schema = self.schema()?;
        Ok(schema
            .groups
            .iter()
            .map(|g| Group { engine: self.engine.clone(), device: self.id, group_id: g.id })
            .collect())
    }

    /// Groups belonging to a particular menu / access level.
    pub fn tab(&self, menu: Menu) -> Result<Vec<Group>> {
        let schema = self.schema()?;
        Ok(schema
            .groups
            .iter()
            .filter(|g| g.menu == menu)
            .map(|g| Group { engine: self.engine.clone(), device: self.id, group_id: g.id })
            .collect())
    }

    /// A handle to a field by its global index.
    pub fn field(&self, index: i32) -> Field {
        Field { engine: self.engine.clone(), device: self.id, index }
    }
}

/// A group of fields within a device.
#[derive(Clone)]
pub struct Group {
    engine: Arc<Engine>,
    device: u32,
    group_id: i32,
}

impl Group {
    fn info(&self) -> Result<GroupInfo> {
        self.engine.ensure_schema(self.device)?;
        self.engine
            .state
            .schema(self.device)
            .and_then(|s| s.groups.into_iter().find(|g| g.id == self.group_id))
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
            .map(|f| Field { engine: self.engine.clone(), device: self.device, index: f.index })
            .collect())
    }
}

/// A single field of a device.
#[derive(Clone)]
pub struct Field {
    engine: Arc<Engine>,
    device: u32,
    index: i32,
}

impl Field {
    fn info(&self) -> Result<FieldInfo> {
        self.engine.ensure_schema(self.device)?;
        self.engine
            .state
            .schema(self.device)
            .and_then(|s| s.field(self.index).cloned())
            .ok_or(Error::FieldNotAvailable(self.index))
    }

    /// Field index.
    pub fn index(&self) -> i32 {
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
        self.engine.read(self.device, self.index, self.engine.config.max_age)
    }

    /// Write a value; returns the value observed after the write.
    pub fn set(&self, value: Value) -> Result<Value> {
        if !self.info()?.writeable {
            return Err(Error::ReadOnly);
        }
        let wv = match value {
            Value::Boolean(b) => WriteValue::Bool(b),
            Value::Float(f) => WriteValue::Float(f),
            Value::List { index, .. } => WriteValue::ListIndex(index),
            _ => return Err(Error::WrongType { expected: "Boolean|Float|List" }),
        };
        self.engine.write(self.device, self.index, wv)
    }

    /// Subscribe to live updates of just this field.
    pub fn subscribe(&self, interval: Duration, change_only: bool) -> Subscription {
        let (id, rx) = self.engine.subscribe(self.device, vec![self.index], interval, change_only);
        Subscription { engine: self.engine.clone(), id, rx }
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
