//! Error and result types for the public API.

/// Convenience result alias used throughout the crate's public API.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by the MasterBus API.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The CAN interface could not be opened or is not usable.
    #[error("connection error: {0}")]
    Connection(String),

    /// No device with the given id is currently known/alive on the bus.
    #[error("device {0} not found")]
    DeviceNotFound(u32),

    /// The requested group/tab does not exist on the device.
    #[error("group {0} not available")]
    GroupNotAvailable(i32),

    /// The requested field does not exist on the device.
    #[error("field {0} not available")]
    FieldNotAvailable(i32),

    /// Schema/value not yet discovered or no value has arrived in time.
    #[error("data not ready")]
    NotReady,

    /// A request to the device timed out.
    #[error("request timed out")]
    Timeout,

    /// A value of the wrong type was supplied for a write (expected `{expected}`).
    #[error("wrong value type, expected {expected}")]
    WrongType {
        /// The type that was expected for this field.
        expected: &'static str,
    },

    /// The field is not writable.
    #[error("field is read-only")]
    ReadOnly,

    /// A malformed or unexpected protocol frame.
    #[error("protocol error: {0}")]
    Protocol(String),
}
