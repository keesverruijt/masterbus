//! Idiomatic Rust client for the Mastervolt **MasterBus** CAN protocol.
//!
//! See `docs/PROTOCOL.md` in the repository for the wire protocol.
//!
//! The crate is being built up incrementally; this is the foundational layer
//! (value/error types). Discovery, caching, subscriptions and the navigator /
//! channel APIs follow.
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
pub use model::{DeviceSchema, DeviceStatus, FieldInfo, GroupInfo, Menu};
pub use protocol::VisualizationType;
pub use runtime::{Config, DeviceEvent, ValueUpdate};
pub use value::{Date, Time, Value, WriteValue};
