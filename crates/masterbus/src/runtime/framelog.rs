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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Frame lines recorded by [`Capture`]. Shared with every other test in
    /// the binary, since a process has one logger — tests look for their own
    /// line rather than asserting on the whole buffer.
    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static CAPTURE: Capture = Capture;

    /// A logger that keeps frame lines and drops everything else.
    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.target() == "masterbus::frame"
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                LINES.lock().unwrap().push(r.args().to_string());
            }
        }
        fn flush(&self) {}
    }

    fn capture() {
        let _ = log::set_logger(&CAPTURE);
        log::set_max_level(log::LevelFilter::Trace);
    }

    fn recorded(needle: &str) -> Option<String> {
        LINES
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.contains(needle))
            .cloned()
    }

    /// The format is candump-compatible with the semantic tag in a fixed
    /// column — a payload of any length leaves the tag in the same place, so
    /// a trace is readable as columns.
    #[test]
    fn a_frame_line_is_candump_shaped_with_an_aligned_tag() {
        capture();
        frame_log("Tx", 0x18_AB_CD_EF, &[0x17, 0x00]);
        frame_log("Rx", 0x08_AB_CD_E0, &[1, 2, 3, 4, 5, 6, 7, 8]);

        let short = recorded("18ABCDEF").expect("short frame logged");
        let long = recorded("08ABCDE0").expect("long frame logged");

        assert!(short.starts_with("Tx  18ABCDEF   [2]  17 00"), "{short}");
        assert!(short.ends_with("; read-btm1"), "{short}");
        assert!(
            long.starts_with("Rx  08ABCDE0   [8]  01 02 03 04 05 06 07 08"),
            "{long}"
        );
        assert!(long.ends_with("; push-btm1"), "{long}");
        // The whole point of the padding: one tag column.
        assert_eq!(short.find(';'), long.find(';'));
    }

    /// An empty payload still logs (the bus-master heartbeat is 0 bytes).
    #[test]
    fn an_empty_payload_still_logs() {
        capture();
        frame_log("Tx", 0x05_53_A4_93, &[]);
        let line = recorded("0553A493").expect("heartbeat logged");
        assert!(line.contains("[0]"), "{line}");
        assert!(line.ends_with("; poll"), "{line}");
    }

    /// Direction and payload length disambiguate the dual-purpose classes:
    /// `0x18` is a Btm1 read or write, `0x1B` a history query or a Btm3
    /// write, `0x07` a request or a chunk echo.
    #[test]
    fn the_dual_purpose_classes_are_told_apart() {
        assert_eq!(classify("Tx", 0x18, &[0x17, 0x00]), "read-btm1");
        assert_eq!(
            classify("Tx", 0x18, &[0x17, 0x00, 0, 0, 0x80, 0x40]),
            "write-btm1"
        );
        assert_eq!(classify("Rx", 0x18, &[0x17, 0x00]), "loopback-read-btm1");

        assert_eq!(classify("Tx", 0x1B, &[0x30, 0x00]), "history-req");
        assert_eq!(
            classify("Tx", 0x1B, &[0x30, 0, 0, 0, 0x88, 0x41]),
            "write-btm3"
        );
        assert_eq!(classify("Rx", 0x1B, &[0x28, 0x00, 0x00]), "history-resp");

        assert_eq!(classify("Tx", 0x07, &[0x08, 0x19]), "prop-req");
        assert_eq!(
            classify("Rx", 0x07, &[0x30, 0x01, 0x00, 0x00]),
            "prop-resp-chunk"
        );
    }

    /// Every other class has one fixed tag, and an unknown class is marked
    /// rather than mislabelled.
    #[test]
    fn each_remaining_class_has_its_own_tag() {
        for (class, tag) in [
            (0x04, "broadcast"),
            (0x05, "poll"),
            (0x06, "prop-resp"),
            (0x08, "push-btm1"),
            (0x09, "schema-resp"),
            (0x0A, "alarm-resp"),
            (0x0B, "history-or-btm3-ack"),
            (0x0C, "meta-btm3-resp"),
            (0x10, "ack"),
            (0x11, "no-value"),
            (0x19, "schema-req"),
            (0x1A, "alarm-req"),
            (0x1C, "meta-btm3-req"),
            (0x1F, "?"),
        ] {
            assert_eq!(classify("Tx", class, &[]), tag, "class 0x{class:02X}");
            assert_eq!(classify("Rx", class, &[]), tag, "class 0x{class:02X}");
        }
    }
}
