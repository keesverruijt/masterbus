//! Per-frame `log::trace!` calls on `target = "masterbus::frame"`, in a
//! candump-compatible format:
//!
//! ```text
//! Rx  04188EA2   [8]  14 93 A4 03 D5 01 00 00
//! Tx  05000001   [0]
//! ```
//!
//! The `Rx`/`Tx` tag takes candump's iface slot — the engine knows direction
//! but not the underlying SocketCAN iface name (and the USB transport has
//! none). Anything else is bit-identical to `candump`'s default output.
//!
//! Enable with `RUST_LOG=masterbus::frame=trace` (env_logger) or the
//! equivalent for whatever `log`-compatible backend the consuming binary
//! installs. With no logger initialized this is one atomic load and return.

/// Log a frame. `dir` is `"Rx"` (received) or `"Tx"` (transmitted).
pub(crate) fn frame_log(dir: &str, can_id: u32, data: &[u8]) {
    if !log::log_enabled!(target: "masterbus::frame", log::Level::Trace) {
        return;
    }
    let mut hex = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            hex.push(' ');
        }
        hex.push_str(&format!("{b:02X}"));
    }
    log::trace!(
        target: "masterbus::frame",
        "{dir}  {can_id:08X}   [{n}]  {hex}",
        n = data.len(),
    );
}
