//! Shared code for the `masterbus-tools` binaries.
//!
//! The tools were binaries only until the Signal K field mapping became
//! something two of them touch: `masterbus-signalk` reads it to decide what to
//! publish, and `masterbus-tui` edits it. That shared surface lives here.
//!
//! - [`mapping`] — the curated MasterBus → Signal K mapping, keyed on device
//!   serial and field id.
//! - [`units`] — the device-unit → Signal K SI unit and conversion, derived
//!   from the device's own unit rather than stored per field.
//! - [`signalk`] — the Signal K side: the leaf cross-check, boolean leaves
//!   and their truth tables, and how a device value is encoded for the wire.
//! - [`seed`] — path *suggestions*: the bundled per-model database first, then
//!   the old per-class name table, both proposals with no authority.
//! - [`database`] — bundled per-model suggestions keyed on article and field
//!   id, for the models a name table cannot tell apart.

pub mod database;
pub mod mapping;
pub mod seed;
pub mod signalk;
pub mod units;
