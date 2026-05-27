//! Public navigator API: `MasterBus` → `Device` → `Group` → `Field`, plus
//! rate-based subscriptions. Handles are cheap, `Clone`, `'static` (Arc-backed).

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;

use crate::error::{Error, Result};
use crate::model::{AccessLevel, DeviceIdentity, DeviceSchema, DeviceStatus, FieldInfo, GroupInfo, Menu};
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

    /// Groups belonging to a particular menu / access level. Only that menu is
    /// discovered (lazily), not the whole device.
    pub fn tab(&self, menu: Menu) -> Result<Vec<Group>> {
        self.engine.ensure_menu(self.id, menu)?;
        let schema = self.engine.state.schema(self.id).ok_or(Error::NotReady)?;
        Ok(schema
            .groups
            .iter()
            .filter(|g| g.menu == menu)
            .map(|g| Group { engine: self.engine.clone(), device: self.id, group_id: g.id })
            .collect())
    }

    /// Discover (only) `menu` and return its groups as raw [`GroupInfo`]
    /// (name, fields with metadata). Convenient for building a UI tab without
    /// triggering full-device discovery.
    pub fn tab_info(&self, menu: Menu) -> Result<Vec<GroupInfo>> {
        self.engine.ensure_menu(self.id, menu)?;
        let schema = self.engine.state.schema(self.id).ok_or(Error::NotReady)?;
        Ok(schema.groups.into_iter().filter(|g| g.menu == menu).collect())
    }

    /// A handle to a field by its global index.
    pub fn field(&self, index: i32) -> Field {
        Field { engine: self.engine.clone(), device: self.id, index }
    }

    /// Flat probe of the device's entire field-index space, ignoring the
    /// per-menu group counts (which lie on some devices — see PROTOCOL.md
    /// §4.3 + FINDINGS for the Magic-class Nav Chg). Returns every field
    /// index that responded to a shadow query, with no group structure.
    pub fn all_fields(&self) -> Result<Vec<FieldInfo>> {
        self.engine.ensure_all_fields(self.id)?;
        self.engine.state.all_fields(self.id).ok_or(Error::NotReady)
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

    /// Log this device in at `level`. Returns the level the device reports
    /// after the request (which on success equals `level`).
    ///
    /// `login(AccessLevel::EndUser)` is equivalent to [`Self::logout`].
    pub fn login(&self, level: AccessLevel) -> Result<AccessLevel> {
        self.engine.set_access_level(self.id, level)
    }

    /// Log out of the device (return to `AccessLevel::EndUser`). Wire form is
    /// a 4-byte request with no code payload (PROTOCOL.md §4.5).
    pub fn logout(&self) -> Result<AccessLevel> {
        self.engine.set_access_level(self.id, AccessLevel::EndUser)
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
    /// The field's full schema entry (name, unit, viz type, writeable flag,
    /// numeric bounds, and list/enum option labels). Pairs with [`value`](Self::value)
    /// so a caller can resolve a list value's index to its label.
    pub fn info(&self) -> Result<FieldInfo> {
        self.engine.ensure_field(self.device, self.index)?;
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
        self.engine.write(self.device, self.index, wv)
    }

    /// Subscribe to live updates of just this field.
    pub fn subscribe(&self, interval: Duration, change_only: bool) -> Subscription {
        let (id, rx) = self.engine.subscribe(self.device, vec![self.index], interval, change_only);
        Subscription { engine: self.engine.clone(), id, rx }
    }
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
        V::Radio | V::DropDown => match value {
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
        // Greyed (read-only display) and Date/Time/Text have no write encoding.
        V::GrayVisualization | V::Date | V::Time | V::Text => wrong("a writable type"),
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
    use crate::value::Date;

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
            write_value_for(VisualizationType::DropDown, Value::List { index: 2, options: vec![] }),
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
        // Non-writable field types are rejected.
        assert!(matches!(
            write_value_for(VisualizationType::Date, Value::Date(Date { day: 1, mon: 1, year: 2026 })),
            Err(Error::WrongType { .. })
        ));
    }
}
