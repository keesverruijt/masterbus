//! C ABI for the `masterbus` crate.
//!
//! **Single-threaded contract**: a context must only be used from one thread.
//! The header `masterbus.h` is generated from this crate via cbindgen.

// The extern "C" surface is added as the core navigator API lands.
