//! An in-process fake MasterBus device, for the runtime tests.
//!
//! [`Device`] is a small state machine that answers the request classes the
//! scheduler emits — property/login (`0x07`), Btm1 values (`0x18`), Btm3
//! values (`0x1B` on the shadow address) — and announces itself the way a
//! real device does. [`FakeBus::start`] wires one to the [`Transport`] trait
//! and runs it on its own thread, so a test drives the *real* reader and
//! scheduler threads over a loopback bus and can assert both on what went out
//! on the wire and on what the engine concluded.
//!
//! A bare [`Device::new`] answers values, strings and login, but knows no
//! schema: every schema and metadata query draws silence, which is the
//! "device won't enumerate" case. Adding groups with [`Device::with_group`]
//! and fields with [`Device::with_field`] and friends makes it enumerable, so
//! the same harness drives the discovery path end to end. Tests that only
//! care about reads, writes or subscriptions skip all that and seed the
//! schema into [`super::State`] directly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};

use crate::error::Result;
use crate::model::{AccessLevel, Channel, FieldId, Menu, field_id};
use crate::protocol::{BTM1_META_ADDR_FLAG, can_class, meta_op};
use crate::transport::{Transport, TransportRx, TransportTx};

/// The fake device's address (a CombiMaster's, from the captures).
pub(crate) const ADDR: u32 = 0x188EA2;

/// How often the device announces itself. A real one broadcasts every second
/// or two; a test shouldn't wait that long to connect.
const BROADCAST_INTERVAL: Duration = Duration::from_millis(10);

/// A raw CAN frame: extended id plus payload.
type Frame = (u32, Vec<u8>);

/// Build a raw extended-CAN id from a class byte and an address.
fn id(class: u8, addr: u32) -> u32 {
    ((class as u32) << 24) | (addr & 0x00_FF_FF_FF)
}

/// One field's metadata, as the per-field opcodes report it.
#[derive(Clone)]
pub(crate) struct FakeField {
    /// String id of the field name (0 = no name).
    pub name_sid: u16,
    /// String id of the unit (0 = none).
    pub unit_sid: u16,
    /// Wire visualization code (meta op `0x02`).
    pub viz: u8,
    /// Meta op `0x07`: the numeric maximum, or the option count for a list.
    pub max: f32,
    /// Meta op `0x0B`.
    pub writeable: bool,
    /// Meta op `0x0D`. A real device answers this op *only* for its eventable
    /// fields, so a `false` here means silence, not a zero byte.
    pub eventable: bool,
    /// String ids of the option labels, for list types.
    pub option_sids: Vec<u16>,
}

/// One group, as the schema channel reports it.
#[derive(Clone)]
pub(crate) struct FakeGroup {
    /// String id of the group name.
    pub name_sid: u16,
    /// Btm1 wire indices of the group's fields, in display order.
    pub fields: Vec<u8>,
}

/// A scriptable MasterBus device.
pub(crate) struct Device {
    /// The device's bus address.
    pub addr: u32,
    /// Device-family byte in the broadcast.
    pub type_code: u8,
    /// Firmware hint (u16 LE) in the broadcast.
    pub firmware: u16,
    /// Current access level, as reported by `0x08 0x19`.
    pub level: AccessLevel,
    /// Level byte → the code that unlocks it. A login with any other code is
    /// ignored: the device stays where it is and still answers with its
    /// current level, which is exactly how a wrong code presents on the wire.
    pub codes: HashMap<u8, f32>,
    /// Btm1 field values, by wire index.
    pub btm1: HashMap<u8, [u8; 4]>,
    /// Btm3 field values, by wire index.
    pub btm3: HashMap<u8, [u8; 4]>,
    /// String table (opcode `0x30` chunk reads and writes).
    pub strings: HashMap<u16, String>,
    /// Property string ids: 1 = article, 2 = serial, 3 = name, 4 = revision.
    pub prop_sids: HashMap<u8, u16>,
    /// `(minor, major)` halves reported by the two `0x82` firmware queries.
    pub fw_parts: [(u8, u8); 2],
    /// Wire indices the device refuses to answer a value read for, on either
    /// channel — the silent-field case a real bus produces under load.
    pub mute: Vec<u8>,
    /// Stop announcing (the device has dropped off the bus).
    pub quiet: bool,
    /// Group counts by `[0x08, selector]` selector byte.
    pub group_counts: HashMap<u8, u16>,
    /// Groups, keyed by (schema request class, gid). Alarm and History have
    /// their own gid namespaces, which is why the class is part of the key.
    pub groups: HashMap<(u8, u8), FakeGroup>,
    /// Per-field metadata, keyed by channel-tagged field id. An index that is
    /// absent here answers nothing at all — an unallocated slot.
    pub meta: HashMap<FieldId, FakeField>,
    /// Next string id handed out by the `with_*` builders.
    next_sid: u16,
}

impl Device {
    /// A device that answers everything with zeroes and has no login codes.
    pub fn new() -> Device {
        Device {
            addr: ADDR,
            type_code: 0x0B,
            firmware: 0x0102,
            level: AccessLevel::EndUser,
            codes: HashMap::new(),
            btm1: HashMap::new(),
            btm3: HashMap::new(),
            strings: HashMap::new(),
            prop_sids: HashMap::new(),
            fw_parts: [(0, 1), (0, 0)],
            mute: Vec::new(),
            quiet: false,
            group_counts: HashMap::new(),
            groups: HashMap::new(),
            meta: HashMap::new(),
            next_sid: 0x100,
        }
    }

    /// Intern a string and return its id.
    fn intern(&mut self, text: &str) -> u16 {
        if text.is_empty() {
            return 0;
        }
        let sid = self.next_sid;
        self.next_sid += 1;
        self.strings.insert(sid, text.to_string());
        sid
    }

    /// Add a group to a menu's schema channel. For the three global-gid menus
    /// the group count reported by `[0x08, selector]` is kept in step, so the
    /// gids a test passes are the global ones discovery will ask for.
    pub fn with_group(mut self, menu: Menu, gid: u8, name: &str, fields: &[u8]) -> Device {
        let name_sid = self.intern(name);
        self.groups.insert(
            (menu.schema_request_class(), gid),
            FakeGroup {
                name_sid,
                fields: fields.to_vec(),
            },
        );
        if matches!(menu, Menu::Monitoring | Menu::Configuration | Menu::Service) {
            *self.group_counts.entry(menu.selector()).or_insert(0) += 1;
        }
        self
    }

    /// Add a plain field's metadata on either channel.
    pub fn with_field(
        mut self,
        fid: FieldId,
        name: &str,
        unit: &str,
        viz: u8,
        writeable: bool,
    ) -> Device {
        let name_sid = self.intern(name);
        let unit_sid = self.intern(unit);
        self.add_meta(
            fid,
            FakeField {
                name_sid,
                unit_sid,
                viz,
                max: 0.0,
                writeable,
                eventable: false,
                option_sids: Vec::new(),
            },
        )
    }

    /// Add a drop-down field whose option labels resolve through the string
    /// table, the way a real enum does.
    pub fn with_list_field(mut self, fid: FieldId, name: &str, options: &[&str]) -> Device {
        let name_sid = self.intern(name);
        let option_sids: Vec<u16> = options.iter().map(|o| self.intern(o)).collect();
        self.add_meta(
            fid,
            FakeField {
                name_sid,
                unit_sid: 0,
                viz: 0x03,
                max: option_sids.len() as f32,
                writeable: true,
                eventable: false,
                option_sids,
            },
        )
    }

    /// Add a field that reports itself as an eventable output (meta op `0x0D`).
    pub fn with_eventable_field(mut self, fid: FieldId, name: &str) -> Device {
        let name_sid = self.intern(name);
        self.add_meta(
            fid,
            FakeField {
                name_sid,
                unit_sid: 0,
                viz: 0x01,
                max: 0.0,
                writeable: true,
                eventable: true,
                option_sids: Vec::new(),
            },
        )
    }

    fn add_meta(mut self, fid: FieldId, f: FakeField) -> Device {
        self.meta.insert(fid, f);
        self
    }

    /// Give the device a full identity: the four property strings plus the
    /// string-table entries they point at.
    pub fn with_identity(mut self, article: &str, serial: &str, name: &str) -> Device {
        for (n, sid, text) in [
            (1u8, 0x10u16, article),
            (2, 0x11, serial),
            (3, 0x12, name),
            (4, 0x13, "3"),
        ] {
            self.prop_sids.insert(n, sid);
            self.strings.insert(sid, text.to_string());
        }
        self
    }

    /// Seed a Btm1 field's value as a little-endian f32.
    pub fn with_btm1(mut self, wire: u8, value: f32) -> Device {
        self.btm1.insert(wire, value.to_le_bytes());
        self
    }

    /// Seed a Btm3 field's value as a little-endian f32.
    pub fn with_btm3(mut self, wire: u8, value: f32) -> Device {
        self.btm3.insert(wire, value.to_le_bytes());
        self
    }

    /// The periodic class-`0x04` self-announcement.
    fn broadcast(&self) -> Frame {
        let [lo, hi] = self.firmware.to_le_bytes();
        (
            id(can_class::DEVICE_BROADCAST, self.addr),
            vec![self.type_code, 0, 0, 0, lo, hi],
        )
    }

    /// Answer one frame addressed to this device. Frames for another address,
    /// or on a class this device doesn't speak, draw silence.
    pub fn respond(&mut self, raw_id: u32, data: &[u8]) -> Vec<Frame> {
        let class = ((raw_id >> 24) & 0x1F) as u8;
        let addr = raw_id & 0x00_FF_FF_FF;
        if addr & !BTM1_META_ADDR_FLAG != self.addr {
            return Vec::new();
        }
        let shadow = addr & BTM1_META_ADDR_FLAG != 0;
        match (class, shadow) {
            (can_class::PROPERTY_REQ, false) => self.property(data),
            (can_class::MONITORING_REQ, false) => self.btm1_value(data),
            // Btm1 per-field metadata lives on the shadow address; the reply
            // comes back on class 0x08 at that same address.
            (can_class::MONITORING_REQ, true) => {
                let reply = id(can_class::MONITORING_DATA, addr);
                self.metadata(Channel::Btm1, data, reply)
            }
            // Btm3 metadata is class 0x1C on the real address, answered on 0x0C.
            (can_class::BTM3_META_REQ, false) => {
                let reply = id(can_class::BTM3_META_DATA, addr);
                self.metadata(Channel::Btm3, data, reply)
            }
            (can_class::SCHEMA_REQ_HISTORY, true) => self.btm3_value(data),
            // The three schema channels. Class 0x1B doubles as the Btm3 value
            // carrier, which is why only the real-address form lands here.
            (
                can_class::SCHEMA_REQ | can_class::SCHEMA_REQ_ALARM | can_class::SCHEMA_REQ_HISTORY,
                false,
            ) => self.schema(class, data),
            _ => Vec::new(),
        }
    }

    /// The three schema channels: group name, field count, field id. An
    /// unregistered gid answers nothing, which is how probe-and-stop on the
    /// Alarm and History namespaces terminates.
    fn schema(&mut self, class: u8, data: &[u8]) -> Vec<Frame> {
        let reply = id(class - 0x10, self.addr);
        let group = |d: &Device, gid: u8| d.groups.get(&(class, gid)).cloned();
        match data {
            [0x28, gid, _] => match group(self, *gid) {
                Some(g) => {
                    let [lo, hi] = g.name_sid.to_le_bytes();
                    vec![(reply, vec![0x28, *gid, 0x00, 0x00, lo, hi])]
                }
                None => Vec::new(),
            },
            [0x07, gid, _] => match group(self, *gid) {
                Some(g) => {
                    let mut d = vec![0x07, *gid, 0x00, 0x00];
                    d.extend_from_slice(&(g.fields.len() as f32).to_le_bytes());
                    vec![(reply, d)]
                }
                None => Vec::new(),
            },
            [0x03, gid, _, idx] => {
                match group(self, *gid).and_then(|g| g.fields.get(*idx as usize).copied()) {
                    Some(wire) => vec![(reply, vec![0x03, *gid, 0x00, *idx, wire, 0x00])],
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// Per-field metadata, one opcode per request. Both channels speak the
    /// same opcode set over independent field-index namespaces; an index with
    /// no metadata stays silent, which is what the existence probe reads as
    /// "no field here".
    fn metadata(&mut self, channel: Channel, data: &[u8], reply: u32) -> Vec<Frame> {
        let tag = |wire: u8| match channel {
            Channel::Btm1 => field_id::btm1(wire),
            Channel::Btm3 => field_id::btm3(wire),
        };
        // Option query: `[0x26, wire, 0x00, opt]`.
        if let [meta_op::OPTION, wire, _, opt] = data {
            let sid = self
                .meta
                .get(&tag(*wire))
                .and_then(|f| f.option_sids.get(*opt as usize).copied());
            return match sid {
                Some(sid) => {
                    let [lo, hi] = sid.to_le_bytes();
                    vec![(reply, vec![meta_op::OPTION, *wire, 0x00, *opt, lo, hi])]
                }
                None => Vec::new(),
            };
        }
        let [op, lo, hi] = data else {
            return Vec::new();
        };
        // The wire index is a u16 in the request; every device seen so far
        // keeps it inside a byte.
        let wire = u16::from_le_bytes([*lo, *hi]);
        let Some(f) = self.meta.get(&tag(wire as u8)).cloned() else {
            return Vec::new();
        };
        if wire > u8::MAX as u16 {
            return Vec::new();
        }
        let head = vec![*op, *lo, *hi, 0x00];
        let with = |tail: &[u8]| {
            let mut d = head.clone();
            d.extend_from_slice(tail);
            vec![(reply, d)]
        };
        match *op {
            meta_op::NAME => with(&f.name_sid.to_le_bytes()),
            meta_op::UNIT => with(&f.unit_sid.to_le_bytes()),
            meta_op::VIZ => with(&[f.viz]),
            meta_op::MAX => with(&f.max.to_le_bytes()),
            meta_op::WRITEABLE => with(&[f.writeable as u8]),
            // Answered only for eventable fields — silence otherwise.
            meta_op::EVENTABLE if f.eventable => with(&[1]),
            _ => Vec::new(),
        }
    }

    /// Class `0x07`: login/logout, firmware, property string ids, and string
    /// chunks in both directions. Replies land on class `0x06`.
    fn property(&mut self, data: &[u8]) -> Vec<Frame> {
        let addr = self.addr;
        let info = move |d: Vec<u8>| vec![(id(can_class::PROPERTY_INFO, addr), d)];
        match data {
            // Read the current level.
            [0x08, 0x19] => info(vec![0x08, 0x19, self.level.level_byte(), 0x00]),
            // Logout — header only, no code.
            [0x08, 0x19, _, _] => {
                self.level = AccessLevel::EndUser;
                info(vec![0x08, 0x19, self.level.level_byte(), 0x00])
            }
            // Login at `lvl` with an f32 code.
            [0x08, 0x19, lvl, _, c0, c1, c2, c3] => {
                let code = f32::from_le_bytes([*c0, *c1, *c2, *c3]);
                if self.codes.get(lvl) == Some(&code)
                    && let Some(l) = AccessLevel::from_byte(*lvl)
                {
                    self.level = l;
                }
                info(vec![0x08, 0x19, self.level.level_byte(), 0x00])
            }
            // Group count for a menu selector. `[0x08, 0x19]` is the login
            // register and is matched above, so it never reaches here.
            [0x08, sel] => {
                let [lo, hi] = self
                    .group_counts
                    .get(sel)
                    .copied()
                    .unwrap_or(0)
                    .to_le_bytes();
                info(vec![0x08, *sel, lo, hi])
            }
            [0x82, n] => {
                let (minor, major) = self.fw_parts[(*n as usize).min(1)];
                info(vec![0x82, *n, minor, major])
            }
            [0x09, n] => {
                let [lo, hi] = self.prop_sids.get(n).copied().unwrap_or(0).to_le_bytes();
                info(vec![0x09, *n, lo, hi])
            }
            // String read: up to four chars per round trip, NUL-terminated
            // once the string runs out.
            [0x30, lo, hi, seq] => {
                let sid = u16::from_le_bytes([*lo, *hi]);
                let s = self.strings.get(&sid).cloned().unwrap_or_default();
                let rest = s.as_bytes().get(*seq as usize * 4..).unwrap_or(&[]);
                let chunk = &rest[..rest.len().min(4)];
                let mut d = vec![0x30, *lo, *hi, *seq];
                d.extend_from_slice(chunk);
                if chunk.len() < 4 {
                    d.push(0x00);
                }
                info(d)
            }
            // String write: same opcode with a payload. Acked by echoing the
            // header back.
            [0x30, lo, hi, seq, chars @ ..] => {
                let sid = u16::from_le_bytes([*lo, *hi]);
                let text = self.strings.entry(sid).or_default();
                if *seq == 0 {
                    text.clear();
                }
                for &c in chars {
                    if c == 0 {
                        break;
                    }
                    text.push(c as char);
                }
                info(vec![0x30, *lo, *hi, *seq])
            }
            _ => Vec::new(),
        }
    }

    /// Class `0x18` on the real address: a two-byte read, or a six-byte write.
    /// A read is answered on class `0x08`; a write is silent, as on the wire —
    /// the engine confirms it with a follow-up read.
    fn btm1_value(&mut self, data: &[u8]) -> Vec<Frame> {
        match data {
            [f, tab] => {
                if self.mute.contains(f) {
                    return Vec::new();
                }
                let v = self.btm1.get(f).copied().unwrap_or_default();
                let mut d = vec![*f, *tab];
                d.extend_from_slice(&v);
                vec![(id(can_class::MONITORING_DATA, self.addr), d)]
            }
            [f, _tab, v0, v1, v2, v3] => {
                self.btm1.insert(*f, [*v0, *v1, *v2, *v3]);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Class `0x1B` on the shadow address: a headerless read or write. Both
    /// are answered on class `0x0B` at the same address — a read with the
    /// current value, a write with the value just stored (the ack).
    fn btm3_value(&mut self, data: &[u8]) -> Vec<Frame> {
        let carrier = id(
            can_class::SCHEMA_DATA_HISTORY,
            self.addr | BTM1_META_ADDR_FLAG,
        );
        match data {
            [lo, hi] => {
                if self.mute.contains(lo) {
                    return Vec::new();
                }
                let v = self.btm3.get(lo).copied().unwrap_or_default();
                let mut d = vec![*lo, *hi];
                d.extend_from_slice(&v);
                vec![(carrier, d)]
            }
            [lo, hi, v0, v1, v2, v3] => {
                self.btm3.insert(*lo, [*v0, *v1, *v2, *v3]);
                vec![(carrier, vec![*lo, *hi, *v0, *v1, *v2, *v3])]
            }
            _ => Vec::new(),
        }
    }
}

/// A running fake bus: the device thread plus a record of everything the
/// engine transmitted. Dropping it stops the thread.
pub(crate) struct FakeBus {
    sent: Arc<Mutex<Vec<Frame>>>,
    /// The device itself, for mid-test inspection and reconfiguration.
    pub device: Arc<Mutex<Device>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeBus {
    /// Start the device thread and hand back the transport to connect over.
    pub fn start(device: Device) -> (FakeBus, Box<dyn Transport>) {
        let (down_tx, down_rx) = unbounded::<Frame>();
        let (up_tx, up_rx) = unbounded::<Frame>();
        let device = Arc::new(Mutex::new(device));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread = {
            let (device, sent, stop) = (device.clone(), sent.clone(), stop.clone());
            std::thread::Builder::new()
                .name("fake-device".into())
                .spawn(move || {
                    let mut next_broadcast = Instant::now();
                    while !stop.load(Ordering::Relaxed) {
                        if Instant::now() >= next_broadcast {
                            let frame = {
                                let d = device.lock().unwrap();
                                (!d.quiet).then(|| d.broadcast())
                            };
                            if let Some(f) = frame
                                && up_tx.send(f).is_err()
                            {
                                return;
                            }
                            next_broadcast = Instant::now() + BROADCAST_INTERVAL;
                        }
                        match down_rx.recv_timeout(Duration::from_millis(1)) {
                            Ok((raw_id, data)) => {
                                sent.lock().unwrap().push((raw_id, data.clone()));
                                let replies = device.lock().unwrap().respond(raw_id, &data);
                                for r in replies {
                                    if up_tx.send(r).is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                })
                .expect("spawn fake device")
        };

        (
            FakeBus {
                sent,
                device,
                stop,
                thread: Some(thread),
            },
            Box::new(FakeTransport { up_rx, down_tx }),
        )
    }

    /// Every frame the engine has transmitted, in order.
    pub fn sent(&self) -> Vec<Frame> {
        self.sent.lock().unwrap().clone()
    }

    /// Transmitted frames of one CAN class.
    pub fn sent_class(&self, class: u8) -> Vec<Frame> {
        self.sent()
            .into_iter()
            .filter(|(raw_id, _)| ((raw_id >> 24) & 0x1F) as u8 == class)
            .collect()
    }

    /// Edit the device mid-test (change a value, go quiet, add a login code).
    pub fn with_device(&self, f: impl FnOnce(&mut Device)) {
        f(&mut self.device.lock().unwrap());
    }
}

impl Drop for FakeBus {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct FakeTransport {
    up_rx: Receiver<Frame>,
    down_tx: Sender<Frame>,
}

impl Transport for FakeTransport {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (Box::new(FakeRx(self.up_rx)), Box::new(FakeTx(self.down_tx)))
    }
}

struct FakeRx(Receiver<Frame>);

impl TransportRx for FakeRx {
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        match self.0.recv_timeout(timeout) {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            // The device thread has stopped (the test is finishing). Idle for
            // the timeout rather than spinning the reader thread.
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(timeout);
                Ok(None)
            }
        }
    }
}

struct FakeTx(Sender<Frame>);

impl TransportTx for FakeTx {
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
        let _ = self.0.send((can_id, data.to_vec()));
        Ok(())
    }
}
