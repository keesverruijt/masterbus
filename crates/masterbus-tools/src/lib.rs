//! Shared code for the `masterbus-tools` binaries.
//!
//! The tools were binaries only until the Signal K field mapping became
//! something two of them touch: `masterbus-signalk` reads it to decide what to
//! publish, and `masterbus-tui` edits it. That shared surface lives here.
//!
//! - [`mapping`] — the curated MasterBus → Signal K mapping, keyed on device
//!   serial and field id.
//! - [`units`] — the device-unit → Signal K SI conversion, derived from the
//!   pair of units rather than stored per field.
//! - [`signalk`] — the Signal K vocabulary: which SI unit each path leaf
//!   carries, and how a device value is encoded for the wire.
//! - [`seed`] — path *suggestions* from the old per-class name table, kept as
//!   a proposal source with no authority.

pub mod mapping;
pub mod seed;
pub mod signalk;
pub mod units;
