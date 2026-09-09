//! Shared code for the `masterbus-tools` binaries.
//!
//! The tools were binaries only until the Signal K field mapping became
//! something two of them touch: `masterbus-signalk` reads it to decide what to
//! publish, and `masterbus-tui` edits it. That shared surface lives here.
//!
//! - [`units`] — the device-unit → Signal K SI conversion, derived from the
//!   pair of units rather than stored per field.
//! - [`signalk`] — the Signal K vocabulary: which SI unit each path leaf
//!   carries.

pub mod signalk;
pub mod units;
