//! Per-frame `log::trace!` calls on `target = "masterbus::frame"`, in a
//! candump-compatible format with a trailing semantic tag so reads, writes,
//! pushes, acks, and discovery requests are immediately distinguishable:
//!
//! ```text
//! Tx  05000001   [0]                              ; poll
//! Tx  18188EA2   [2]  17 00                       ; read-btm1
//! Tx  18188EA2   [6]  17 00 00 00 80 40           ; write-btm1
//! Rx  08188EA2   [8]  14 93 A4 03 D5 01 00 00     ; push-btm1
//! Rx  10188EA2   [4]  17 00 00 00                 ; ack
//! ```
//!
//! Enable with `RUST_LOG=masterbus::frame=trace`. With no logger initialized
//! this is one atomic load and a return.

/// Log a frame. `dir` is `"Rx"` (received) or `"Tx"` (transmitted).
pub(crate) fn frame_log(dir: &str, can_id: u32, data: &[u8]) {
    if !log::log_enabled!(target: "masterbus::frame", log::Level::Trace) {
        return;
    }
    let class = ((can_id >> 24) & 0x1F) as u8;
    let mut hex = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            hex.push(' ');
        }
        hex.push_str(&format!("{b:02X}"));
    }
    // Pad the hex column to a fixed width so the tag stays right-aligned at
    // column ~46. Max payload is 8 bytes (`8 * 3 - 1 = 23` chars).
    let padded = format!("{hex:<23}");
    log::trace!(
        target: "masterbus::frame",
        "{dir}  {can_id:08X}   [{n}]  {padded} ; {tag}",
        n = data.len(),
        tag = classify(dir, class, data),
    );
}

/// Short semantic tag for a frame. Direction + class + payload length together
/// disambiguate reads from writes (Btm1 and Btm3 both reuse one class for
/// both, with the payload length distinguishing them).
fn classify(dir: &str, class: u8, data: &[u8]) -> &'static str {
    let is_tx = dir == "Tx";
    let n = data.len();
    match class {
        0x04 => "broadcast",
        0x05 => "poll",
        0x06 => "prop-resp",
        // class 0x07 is dual-purpose: Tx → property/login/string-chunk req
        // (which is the "string-write" carrier too); Rx → chunked-string echo.
        0x07 => {
            if is_tx {
                "prop-req"
            } else {
                "prop-resp-chunk"
            }
        }
        0x08 => "push-btm1",
        0x09 => "schema-resp",
        0x0A => "alarm-resp",
        // class 0x0B is the Btm3 write-ack on Rx, and history schema-resp on Rx.
        0x0B => "history-or-btm3-ack",
        0x0C => "meta-btm3-resp",
        0x10 => "ack",
        0x11 => "no-value",
        // class 0x18: Tx 2-byte payload is a read; longer is a write (set_*).
        0x18 => {
            if is_tx {
                if n <= 2 { "read-btm1" } else { "write-btm1" }
            } else {
                "loopback-read-btm1"
            }
        }
        0x19 => "schema-req",
        0x1A => "alarm-req",
        // class 0x1B: Tx 3-byte schema query vs 6-byte Btm3 write (FIID + f32).
        0x1B => {
            if is_tx {
                if n >= 5 { "write-btm3" } else { "history-req" }
            } else {
                "history-resp"
            }
        }
        0x1C => "meta-btm3-req",
        _ => "?",
    }
}
