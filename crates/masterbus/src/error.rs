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

    /// A string-write payload violated the device-side constraint: editable
    /// strings on MasterBus are limited to 16 bytes of printable ASCII (no
    /// embedded NUL — the NUL is the on-wire terminator).
    #[error("string must be ≤ 16 printable-ASCII chars (got {got} chars; first invalid: {issue})")]
    InvalidText {
        /// Length of the offending string.
        got: usize,
        /// What was wrong: `"too long"` or a description of the bad byte.
        issue: String,
    },

    /// A malformed or unexpected protocol frame.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// A relay-style boolean write needs to follow up with a commit token at
    /// the adjacent hidden register at field+1, but that register is occupied
    /// by a named, user-facing field — sending the commit there would clobber
    /// it. Refused rather than silently failing to actuate.
    #[error(
        "commit-token slot at field 0x{cmd_field:04X} is occupied by named field \"{cmd_field_name}\" \
         — relay-style boolean write to field 0x{field:04X} would not actuate"
    )]
    CommitFieldOccupied {
        /// The field the user tried to write.
        field: i32,
        /// The adjacent register where the commit token would have gone.
        cmd_field: i32,
        /// The user-facing name of the field that's in the way.
        cmd_field_name: String,
    },
}
