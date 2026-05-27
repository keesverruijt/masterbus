//! Idiomatic Rust client for the Mastervolt **MasterBus** CAN-bus protocol.
//!
//! MasterBus is the CAN network used by Mastervolt marine power equipment —
//! inverter/chargers, lithium and lead batteries, alternator regulators,
//! solar/charge controllers, switch panels and displays. This crate speaks the
//! protocol directly: it tracks which devices are present, discovers their
//! schema (the groups and fields each device exposes), reads and writes those
//! fields, and streams live values — without the vendor's closed library.
//!
//! # Quick start
//!
//! The **navigator** API is a small tree of cheap, clonable, `'static` handles:
//! [`MasterBus`] → [`Device`] → [`Group`] → [`Field`].
//!
//! ```no_run
//! # #[cfg(target_os = "linux")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use masterbus::{Config, MasterBus, Menu, Value};
//!
//! // Connect over a Linux SocketCAN interface. `connect` returns as soon as one
//! // device is heard — no multi-second enumeration wait.
//! let bus = MasterBus::socketcan("can0", Config::default())?;
//!
//! // Browse the monitoring tab of every device currently on the bus.
//! for device in bus.devices() {
//!     println!("{} (id {})", device.name()?, device.id());
//!     for group in device.tab(Menu::Monitoring)? {
//!         for field in group.fields()? {
//!             println!("  {:<24} {:?}", field.name()?, field.value()?);
//!         }
//!     }
//! }
//!
//! // Write a value. The supplied `Value` is checked against the field's schema
//! // type, and the value observed after the write is returned.
//! let device = bus.device(0x188EE2);
//! let applied = device.field(23).set(Value::Float(6.0))?;
//! println!("AC input limit is now {applied:?}");
//! # Ok(()) }
//! # #[cfg(not(target_os = "linux"))]
//! # fn main() {}
//! ```
//!
//! # Two API surfaces over one engine
//!
//! - **Navigator** (above): blocking and ergonomic, for scripts and one-shot
//!   tools. [`Field::value`] returns a cached value when it is fresh enough
//!   (see [`Config::max_age`]) and otherwise polls the bus; [`Field::set`]
//!   writes and confirms by observing the resulting value, so a rejected write
//!   is reported truthfully.
//! - **Channel / event**: [`MasterBus::device_events`] streams device presence
//!   ([`DeviceEvent`]), and [`MasterBus::subscribe`] returns a [`Subscription`]
//!   that delivers [`ValueUpdate`]s at a chosen rate. Both are
//!   `crossbeam-channel` receivers, so a TUI or daemon can `select!` bus events
//!   alongside its own (terminal input, timers, …).
//!
//! # Discovery and caching
//!
//! Nothing is enumerated until you ask for it:
//!
//! - **Identity** (name / article / serial / firmware) is the cheap half of
//!   discovery and is fetched on demand — e.g. [`Device::name`].
//! - **Schema** is discovered **lazily per menu** ([`Menu::Monitoring`],
//!   [`Menu::Configuration`], [`Menu::Service`]): [`Device::tab`] discovers just
//!   that menu, while [`Device::groups`]/[`Device::schema`] discover all of them.
//! - An optional on-disk cache ([`Config::cache_path`]) persists each device's
//!   schema across runs (keyed per device, by serial), so a long-running daemon
//!   discovers a device once and loads it from cache thereafter. Schemas are
//!   cached per device rather than shared by article/firmware, since same-model
//!   devices can differ (e.g. one battery in a cluster exposes an extra group).
//!
//! All bus access is serialized on a single scheduler thread and paced to a bus
//! budget, so the crate coexists with the boat's real Mastervolt masters.
//!
//! # Transports
//!
//! The engine runs over any [`transport::Transport`]:
//!
//! - `MasterBus::socketcan` — Linux SocketCAN, built in on Linux.
//! - `MasterBus::usb` — the Mastervolt USB link (cross-platform, HID).
//! - [`MasterBus::with_transport`] — bring your own [`transport::Transport`].
//!
//! # Values
//!
//! Field values are a single [`Value`] enum (float, boolean, list selection,
//! [`Date`], [`Time`], device reference, …). Writes are expressed as a [`Value`]
//! and validated against the field's [`VisualizationType`] before anything is
//! sent.
//!
//! # Protocol reference
//!
//! The wire protocol — frame classes, the date/time encodings, the
//! per-field metadata opcodes, writability, and more — is documented in
//! [`docs/PROTOCOL.md`](https://github.com/keesverruijt/masterbus/blob/main/docs/PROTOCOL.md).
#![warn(missing_docs)]
#![allow(dead_code)]

pub mod api;
pub mod error;
pub mod model;
pub mod protocol;
mod runtime;
pub mod transport;
pub mod value;

pub use api::{Device, Field, Group, MasterBus, Subscription};
pub use error::{Error, Result};
pub use model::{AccessLevel, DeviceIdentity, DeviceSchema, DeviceStatus, FieldInfo, GroupInfo, Menu};
pub use protocol::VisualizationType;
pub use runtime::{Config, DeviceEvent, ValueUpdate};
pub use value::{Date, Time, Value, WriteValue};
