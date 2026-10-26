//! A transport that opens its link again when it goes away.
//!
//! The USB link can be unplugged and plugged back in, and then it is a new
//! device: on Linux a different `/dev/hidrawN`, and the old descriptor only
//! ever reports a hang-up (#39). So when a receive fails, [`Reconnecting`]
//! drops both halves of the link it has, which closes it, and asks its
//! opener for a fresh one every `retry` until it gets one. The engine above
//! keeps its state; the devices announce themselves again once frames flow.
//!
//! The receive half does the reopening, because it is the half that notices:
//! it is always waiting on the link, while the transmit half only speaks now
//! and then. The two share a slot holding the current transmit half, which
//! the receive half empties when the link is lost and fills when it is back.
//! A frame sent while the slot is empty fails at once and is dropped; the
//! scheduler doesn't retry sends, it waits for answers, so it neither spins
//! nor blocks.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{LinkState, Transport, TransportRx, TransportTx};
use crate::error::{Error, Result};

type Frame = (u32, Vec<u8>);
type Opener = Box<dyn FnMut() -> Result<Box<dyn Transport>> + Send>;
type TxSlot = Arc<Mutex<Option<Box<dyn TransportTx>>>>;

/// A transport over a link that `reopen` can open again after it is lost.
pub(crate) struct Reconnecting {
    what: &'static str,
    first: Box<dyn Transport>,
    reopen: Opener,
    retry: Duration,
    link: LinkState,
}

impl Reconnecting {
    /// `first` is the link as opened at start; `reopen` opens it again the
    /// same way. `what` names the link in the log ("USB link").
    pub(crate) fn new(
        what: &'static str,
        first: Box<dyn Transport>,
        reopen: impl FnMut() -> Result<Box<dyn Transport>> + Send + 'static,
        retry: Duration,
    ) -> Self {
        Reconnecting {
            what,
            first,
            reopen: Box::new(reopen),
            retry,
            link: LinkState::new(),
        }
    }
}

impl Transport for Reconnecting {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        let (rx, tx) = self.first.split();
        let slot: TxSlot = Arc::new(Mutex::new(Some(tx)));
        (
            Box::new(Rx {
                what: self.what,
                inner: Some(rx),
                slot: slot.clone(),
                reopen: self.reopen,
                retry: self.retry,
                next_try: Instant::now(),
                opened: Instant::now(),
                link: self.link,
            }),
            Box::new(Tx {
                what: self.what,
                slot,
            }),
        )
    }

    fn link(&self) -> LinkState {
        self.link.clone()
    }
}

struct Rx {
    what: &'static str,
    /// The current link's receive half; `None` while the link is lost.
    inner: Option<Box<dyn TransportRx>>,
    slot: TxSlot,
    reopen: Opener,
    retry: Duration,
    /// When to try to open the link again, while it is lost.
    next_try: Instant,
    /// When the current link was opened. A link lost within `retry` of
    /// opening waits out the rest before the next try, so one that opens
    /// but fails at once is not reopened several times a second.
    opened: Instant,
    link: LinkState,
}

impl TransportRx for Rx {
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        if let Some(rx) = &mut self.inner {
            return match rx.recv(timeout) {
                Err(e) => {
                    // Close both halves: the descriptor of a removed device
                    // reports a hang-up for ever, and holding it would keep
                    // the old node alive.
                    self.inner = None;
                    *self.slot.lock().unwrap() = None;
                    self.link.set(false);
                    self.next_try = (self.opened + self.retry).max(Instant::now());
                    log::warn!(
                        "{} lost ({e}); looking for it every {:?}",
                        self.what,
                        self.retry
                    );
                    Err(e)
                }
                ok => ok,
            };
        }

        let now = Instant::now();
        if now >= self.next_try {
            match (self.reopen)() {
                Ok(t) => {
                    let (rx, tx) = t.split();
                    self.inner = Some(rx);
                    *self.slot.lock().unwrap() = Some(tx);
                    self.link.set(true);
                    self.opened = now;
                    log::info!("{} back", self.what);
                    return Ok(None);
                }
                Err(e) => {
                    log::debug!("{} not back yet: {e}", self.what);
                    self.next_try = now + self.retry;
                }
            }
        }
        // Wait as a receive would, so no caller spins while the link is gone.
        std::thread::sleep(timeout.min(self.next_try.saturating_duration_since(now)));
        Ok(None)
    }
}

struct Tx {
    what: &'static str,
    slot: TxSlot,
}

impl TransportTx for Tx {
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
        match self.slot.lock().unwrap().as_mut() {
            Some(tx) => tx.send(can_id, data),
            None => Err(Error::Connection(format!("the {} is lost", self.what))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crossbeam_channel::{Receiver, Sender, unbounded};

    use super::*;

    /// A link that can be pulled: while `pulled` is set its receive fails as
    /// a hung-up USB link does, and it cannot be opened.
    struct Plug {
        pulled: Arc<AtomicBool>,
        incoming: Receiver<Frame>,
        sent: Sender<Frame>,
    }

    struct PlugRx(Arc<AtomicBool>, Receiver<Frame>);
    struct PlugTx(Sender<Frame>);

    impl Transport for Plug {
        fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
            (
                Box::new(PlugRx(self.pulled, self.incoming)),
                Box::new(PlugTx(self.sent)),
            )
        }
    }
    impl TransportRx for PlugRx {
        fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
            if self.0.load(Ordering::Relaxed) {
                return Err(Error::Connection("USB read: the USB link went away".into()));
            }
            Ok(self.1.recv_timeout(timeout).ok())
        }
    }
    impl TransportTx for PlugTx {
        fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
            let _ = self.0.send((can_id, data.to_vec()));
            Ok(())
        }
    }

    struct Rig {
        pulled: Arc<AtomicBool>,
        opens: Arc<AtomicUsize>,
        to_host: Sender<Frame>,
        on_wire: Receiver<Frame>,
        transport: Box<dyn Transport>,
    }

    fn rig(retry: Duration) -> Rig {
        let pulled = Arc::new(AtomicBool::new(false));
        let opens = Arc::new(AtomicUsize::new(0));
        let (to_host, incoming) = unbounded();
        let (sent, on_wire) = unbounded();
        let plug = {
            let (pulled, incoming, sent) = (pulled.clone(), incoming.clone(), sent.clone());
            move || Plug {
                pulled: pulled.clone(),
                incoming: incoming.clone(),
                sent: sent.clone(),
            }
        };
        let first = Box::new(plug());
        let reopen = {
            let (pulled, opens) = (pulled.clone(), opens.clone());
            move || {
                opens.fetch_add(1, Ordering::Relaxed);
                if pulled.load(Ordering::Relaxed) {
                    Err(Error::Connection("no MasterBus USB Link found".into()))
                } else {
                    Ok(Box::new(plug()) as Box<dyn Transport>)
                }
            }
        };
        Rig {
            pulled,
            opens,
            to_host,
            on_wire,
            transport: Box::new(Reconnecting::new("USB link", first, reopen, retry)),
        }
    }

    /// Pulled, the link reports lost, sends fail without blocking, and the
    /// receive half waits instead of spinning; plugged back, it is reopened
    /// and frames flow both ways again.
    #[test]
    fn a_lost_link_is_reopened_and_traffic_resumes() {
        let r = rig(Duration::from_millis(50));
        let link = r.transport.link();
        let (mut rx, mut tx) = r.transport.split();
        let tick = Duration::from_millis(20);

        r.to_host.send((1, vec![1])).unwrap();
        assert_eq!(rx.recv(tick).unwrap(), Some((1, vec![1])));
        tx.send(2, &[2]).unwrap();
        assert_eq!(r.on_wire.try_recv().unwrap(), (2, vec![2]));
        assert!(link.is_connected());

        r.pulled.store(true, Ordering::Relaxed);
        assert!(rx.recv(tick).is_err());
        assert!(!link.is_connected());
        assert!(tx.send(3, &[3]).is_err());
        assert!(r.on_wire.try_recv().is_err());

        // Lost for a while: every receive waits its timeout, and the link
        // is looked for once per retry interval, not once per receive.
        let t0 = Instant::now();
        for _ in 0..10 {
            assert_eq!(rx.recv(tick).unwrap(), None);
        }
        assert!(t0.elapsed() >= tick * 5, "spun: {:?}", t0.elapsed());
        let opens = r.opens.load(Ordering::Relaxed);
        assert!((2..=8).contains(&opens), "{opens} opens");
        assert!(!link.is_connected());

        r.pulled.store(false, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !link.is_connected() {
            assert!(Instant::now() < deadline, "not reopened");
            let _ = rx.recv(tick);
        }
        r.to_host.send((4, vec![4])).unwrap();
        assert_eq!(rx.recv(tick).unwrap(), Some((4, vec![4])));
        tx.send(5, &[5]).unwrap();
        assert_eq!(r.on_wire.try_recv().unwrap(), (5, vec![5]));
    }

    /// A link that opens but fails its first read (a stale node) is reopened
    /// once per retry interval, not on every receive.
    #[test]
    fn a_link_that_fails_at_once_is_reopened_once_per_interval() {
        struct Stale;
        struct StaleRx;
        impl Transport for Stale {
            fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
                (Box::new(StaleRx), Box::new(PlugTx(unbounded().0)))
            }
        }
        impl TransportRx for StaleRx {
            fn recv(&mut self, _: Duration) -> Result<Option<Frame>> {
                Err(Error::Connection("USB read: the USB link went away".into()))
            }
        }
        let opens = Arc::new(AtomicUsize::new(0));
        let reopen = {
            let opens = opens.clone();
            move || {
                opens.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(Stale) as Box<dyn Transport>)
            }
        };
        let retry = Duration::from_millis(100);
        let t: Box<dyn Transport> = Box::new(Reconnecting::new(
            "USB link",
            Box::new(Stale),
            reopen,
            retry,
        ));
        let (mut rx, _tx) = t.split();
        let t0 = Instant::now();
        while t0.elapsed() < retry * 5 {
            // The reader waits out a short poll after an error.
            if rx.recv(Duration::from_millis(10)).is_err() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let n = opens.load(Ordering::Relaxed);
        assert!((3..=6).contains(&n), "{n} opens in five retry intervals");
    }

    // ── the whole engine, over the fake bus ─────────────────────────────────

    /// A fake-bus connection that fails like a hung-up USB link once pulled.
    struct Pullable(Arc<AtomicBool>, Box<dyn Transport>);
    struct PullableRx(Arc<AtomicBool>, Box<dyn TransportRx>);

    impl Transport for Pullable {
        fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
            let (rx, tx) = self.1.split();
            (Box::new(PullableRx(self.0, rx)), tx)
        }
    }
    impl TransportRx for PullableRx {
        fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
            if self.0.load(Ordering::Relaxed) {
                return Err(Error::Connection("USB read: the USB link went away".into()));
            }
            self.1.recv(timeout)
        }
    }

    /// Unplugged, the engine reports the link lost, stops transmitting and
    /// sees the device go offline; plugged back, the link is reopened, the
    /// device is alive again and our heartbeats reach the bus again.
    #[test]
    fn the_engine_rides_out_a_lost_link() {
        use crate::protocol::can_class;
        use crate::runtime::fakebus::{ADDR, Device, FakeBus};
        use crate::runtime::{Config, DeviceEvent, Engine};

        let (bus, first) = FakeBus::start(Device::new());
        let pulled = Arc::new(AtomicBool::new(false));
        let reopen = {
            let (pulled, connect) = (pulled.clone(), bus.connector());
            move || {
                if pulled.load(Ordering::Relaxed) {
                    return Err(Error::Connection("no MasterBus USB Link found".into()));
                }
                Ok(Box::new(Pullable(pulled.clone(), connect())) as Box<dyn Transport>)
            }
        };
        let transport = Reconnecting::new(
            "USB link",
            Box::new(Pullable(pulled.clone(), first)),
            reopen,
            Duration::from_millis(50),
        );
        let config = Config {
            min_send_interval: Duration::ZERO,
            discovery_window: Duration::ZERO,
            discovery_settle: Duration::ZERO,
            connect_timeout: Duration::from_secs(5),
            liveness: Duration::from_millis(100),
            heartbeat_master: Some(0x53A493),
            heartbeat_interval: Duration::from_millis(10),
            cache_path: None,
            ..Config::default()
        };
        let engine = Engine::connect(Box::new(transport), config).expect("connect");
        let events = engine.device_events();
        let heartbeats = || bus.sent_class(can_class::BUS_POLL).len();
        let wait_for = |what: &str, done: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !done() {
                assert!(Instant::now() < deadline, "{what}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };

        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Alive(ADDR))
        ));
        assert!(engine.link_connected());

        pulled.store(true, Ordering::Relaxed);
        wait_for("link reported lost", &|| !engine.link_connected());
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Offline(ADDR))
        ));
        let sent = heartbeats();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(heartbeats(), sent, "sent while the link was lost");

        pulled.store(false, Ordering::Relaxed);
        wait_for("link reopened", &|| engine.link_connected());
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)),
            Ok(DeviceEvent::Alive(ADDR))
        ));
        wait_for("heartbeats again", &|| heartbeats() > sent);
    }
}
