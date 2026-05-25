//! Field value types.

use serde::{Deserialize, Serialize};

/// A decoded MasterBus field value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// IEEE-754 single. (A quiet-NaN on the wire means "unknown".)
    Float(f32),
    /// Calendar date.
    Date(Date),
    /// Duration / clock value.
    Time(Time),
    /// On/off, checkbox, (push/toggle) button.
    Boolean(bool),
    /// Radio / drop-down list selection.
    List {
        /// Selected option index.
        index: i32,
        /// Option labels (may be empty until resolved).
        options: Vec<String>,
    },
    /// Free text.
    Text(String),
    /// Device reference list selection.
    DeviceRef {
        /// Selected index.
        index: i32,
        /// Referenced device ids.
        device_ids: Vec<u32>,
    },
    /// Eventable list selection.
    Eventable {
        /// Selected index.
        index: i32,
        /// Option labels.
        labels: Vec<String>,
    },
    /// The value is genuinely invalid / unavailable.
    Invalid,
}

/// Calendar date (C-compatible layout for the FFI crate).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Date {
    /// Day of month (1-31), or negative for "unknown".
    pub day: i32,
    /// Month (1-12).
    pub mon: i32,
    /// Full year.
    pub year: i32,
}

/// Clock/duration value (C-compatible layout for the FFI crate).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Time {
    /// Seconds (negative for "unknown").
    pub sec: i32,
    /// Minutes.
    pub min: i32,
    /// Hours.
    pub hour: i32,
    /// Whole days.
    pub days: u32,
}

/// A write request, dispatched by [`crate`] from a typed [`Value`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WriteValue {
    /// Boolean write (`[field, tab, 0|1]`).
    Bool(bool),
    /// Float write (`[field, tab, f32 LE]`).
    Float(f32),
    /// List/enum index write (`[field, tab, index]`).
    ListIndex(i32),
}
