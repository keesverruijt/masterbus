//! Shared code for the `masterbus-tools` binaries. **Not a public API.**
//!
//! This library target exists so the binaries in this package can share code:
//! `masterbus-signalk` reads the Signal K field mapping to decide what to
//! publish, and `masterbus-tui` edits it, so the format, the unit conversion
//! and the suggestion tables have to live in one place.
//!
//! It is published only because Cargo has no way to ship a package's binaries
//! without its library. Nothing here carries a stability guarantee: modules,
//! types and signatures may change in any release, including a patch. Do not
//! depend on `masterbus_tools` from another crate. The stable surface of this
//! project is the [`masterbus`](https://docs.rs/masterbus) library and the
//! command-line tools' own arguments and file formats.
//!
//! Hidden from rendered documentation for that reason. Rustdoc still checks it,
//! so intra-doc links here are held to the same standard as everywhere else.
//!
//! - [`mapping`] — the curated MasterBus → Signal K mapping, keyed on device
//!   serial and field id.
//! - [`units`] — the device-unit → Signal K SI conversion, derived from the
//!   pair of units rather than stored per field.
//! - [`signalk`] — the Signal K vocabulary: which SI unit each path leaf
//!   carries, and how a device value is encoded for the wire.
//! - [`seed`] — path *suggestions*: the bundled per-model database first, then
//!   the old per-class name table, both proposals with no authority.
//! - [`database`] — bundled per-model suggestions keyed on article and field
//!   id, for the models a name table cannot tell apart.

#![doc(hidden)]

pub mod database;
pub mod mapping;
pub mod seed;
pub mod signalk;
pub mod units;
