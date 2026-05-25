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

impl Value {
    /// Attach option labels (from the field's schema) to a list/enum value, so a
    /// caller gets both the numeric index and its meaning. A no-op for other
    /// value kinds.
    pub fn with_options(self, options: &[String]) -> Value {
        match self {
            Value::List { index, .. } => Value::List { index, options: options.to_vec() },
            Value::Eventable { index, .. } => Value::Eventable { index, labels: options.to_vec() },
            other => other,
        }
    }

    /// The numeric selection index of a list/enum value, if applicable.
    pub fn index(&self) -> Option<i32> {
        match self {
            Value::List { index, .. }
            | Value::DeviceRef { index, .. }
            | Value::Eventable { index, .. } => Some(*index),
            _ => None,
        }
    }

    /// The human-readable label of a list/enum selection, if the option strings
    /// are known. Pairs with [`index`](Self::index) to get "both the number and
    /// the explanation".
    pub fn label(&self) -> Option<&str> {
        match self {
            Value::List { index, options } => options.get(*index as usize).map(String::as_str),
            Value::Eventable { index, labels } => labels.get(*index as usize).map(String::as_str),
            _ => None,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_carries_index_and_label() {
        let opts = vec!["Off".to_string(), "On".to_string(), "Auto".to_string()];
        let v = Value::List { index: 2, options: vec![] }.with_options(&opts);
        assert_eq!(v.index(), Some(2));
        assert_eq!(v.label(), Some("Auto"));
    }

    #[test]
    fn label_none_for_non_list() {
        assert_eq!(Value::Float(1.0).label(), None);
        assert_eq!(Value::Float(1.0).index(), None);
    }
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
