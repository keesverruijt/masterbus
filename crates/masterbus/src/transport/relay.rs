//! Share one bus connection between processes.
//!
//! The Mastervolt USB link can be opened by one process at a time, and a
//! SocketCAN interface shared by two engines doubles the polling for nothing
//! when one of them is a long-running daemon. So the process that owns the
//! wire ([`share`]) relays every frame it receives to local clients over a
//! loopback TCP socket and puts their frames on the wire. A client
//! ([`RelayTransport`]) is an ordinary [`Transport`], so a whole engine runs
//! on top of it unchanged.
//!
//! This works because MasterBus requests carry no sender address: a request
//! is addressed to the device, and the device answers to the whole bus. Two
//! engines on one wire see each other's answers, which is harmless; each
//! takes what it was waiting for.
//!
//! The owner announces itself in a small file, `relay.json`, in its schema
//! cache folder (or the per-user cache when caching is off): the address and
//! a random token the client must send first. The file is readable by its
//! owner only, so only the same user (or root) can join. [`find`] reads it
//! back and checks that the relay answers.
//!
//! On the socket, after the client's `token\n` and the owner's `OK\n`, each
//! frame is 4 bytes of big-endian CAN id, one length byte and that many data
//! bytes, in both directions.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

use super::{Transport, TransportRx, TransportTx};
use crate::error::{Error, Result};

/// The announcement file's name, in the cache folder.
pub const RELAY_FILE: &str = "relay.json";

type Frame = (u32, Vec<u8>);
type SharedTx = Arc<Mutex<Box<dyn TransportTx>>>;

/// Put `transport` behind a relay listening on `listen` (a loopback address;
/// port 0 picks a free one), and announce it in `announce_dir`. Returns the
/// transport the owner's own engine should use and the address bound.
pub fn share(
    transport: Box<dyn Transport>,
    listen: &str,
    announce_dir: Option<&Path>,
) -> Result<(Box<dyn Transport>, SocketAddr)> {
    let listener = TcpListener::bind(listen)
        .map_err(|e| Error::Connection(format!("relay listen on {listen}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| Error::Connection(format!("relay address: {e}")))?;
    let token = new_token();
    let (mut rx, tx) = transport.split();
    let tx: SharedTx = Arc::new(Mutex::new(tx));
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let (own_tx, own_rx) = unbounded::<Result<Frame>>();

    // The wire's frames go to the owner's engine and to every client.
    {
        let clients = clients.clone();
        std::thread::Builder::new()
            .name("masterbus-relay-rx".into())
            .spawn(move || {
                loop {
                    let r = rx.recv(Duration::from_millis(200));
                    if let Ok(Some((id, data))) = &r {
                        let bytes = encode(*id, data);
                        // A client that cannot keep up is dropped rather
                        // than allowed to stall the owner.
                        clients
                            .lock()
                            .unwrap()
                            .retain_mut(|c| c.write_all(&bytes).is_ok());
                    }
                    match r {
                        Ok(None) => {}
                        r => {
                            if own_tx.send(r.map(|f| f.unwrap())).is_err() {
                                return;
                            }
                        }
                    }
                }
            })
            .map_err(|e| Error::Connection(format!("relay thread: {e}")))?;
    }

    // Clients join here, and each gets a thread putting its frames on the wire.
    {
        let tx = tx.clone();
        let token = token.clone();
        std::thread::Builder::new()
            .name("masterbus-relay-accept".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let tx = tx.clone();
                    let clients = clients.clone();
                    let token = token.clone();
                    std::thread::spawn(move || serve_client(stream, &token, tx, clients));
                }
            })
            .map_err(|e| Error::Connection(format!("relay thread: {e}")))?;
    }

    if let Some(dir) = announce_dir
        .map(Path::to_path_buf)
        .or_else(crate::settings::user_cache_dir)
    {
        match announce(&dir, addr, &token) {
            Ok(path) => log::info!("relay on {addr}, announced in {}", path.display()),
            Err(e) => log::warn!("relay on {addr}, but not announced: {e}"),
        }
    }

    Ok((Box::new(Owned { rx: own_rx, tx }), addr))
}

fn serve_client(stream: TcpStream, token: &str, tx: SharedTx, clients: Arc<Mutex<Vec<TcpStream>>>) {
    let peer = stream.peer_addr().ok();
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim_end() != token {
        log::warn!("relay: refused {peer:?} (wrong token)");
        return;
    }
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = writer.set_write_timeout(Some(Duration::from_millis(500)));
    if writer.write_all(b"OK\n").is_err() {
        return;
    }
    let _ = reader.get_ref().set_read_timeout(None);
    clients.lock().unwrap().push(writer);
    log::info!("relay: client {peer:?} joined");
    while let Ok((id, data)) = read_frame(&mut reader) {
        if let Err(e) = tx.lock().unwrap().send(id, &data) {
            log::debug!("relay: client frame not sent: {e}");
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
    log::info!("relay: client {peer:?} left");
}

/// The owner's own view of the shared wire.
struct Owned {
    rx: Receiver<Result<Frame>>,
    tx: SharedTx,
}

impl Transport for Owned {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (Box::new(OwnedRx(self.rx)), Box::new(OwnedTx(self.tx)))
    }
}

struct OwnedRx(Receiver<Result<Frame>>);

impl TransportRx for OwnedRx {
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        match self.0.recv_timeout(timeout) {
            Ok(r) => r.map(Some),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(Error::Connection("relay stopped".into())),
        }
    }
}

struct OwnedTx(SharedTx);

impl TransportTx for OwnedTx {
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
        self.0.lock().unwrap().send(can_id, data)
    }
}

/// A bus connection borrowed from a process that [`share`]s one.
pub struct RelayTransport {
    rx: Receiver<Frame>,
    stream: TcpStream,
    addr: SocketAddr,
}

impl RelayTransport {
    /// Join the relay at `addr` with its `token`.
    pub fn connect(addr: SocketAddr, token: &str) -> Result<Self> {
        let fail = |e: std::io::Error| Error::Connection(format!("relay {addr}: {e}"));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(fail)?;
        let _ = stream.set_nodelay(true);
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(fail)?;
        (&stream)
            .write_all(format!("{token}\n").as_bytes())
            .map_err(fail)?;
        let mut reader = BufReader::new(stream.try_clone().map_err(fail)?);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(fail)?;
        if line.trim_end() != "OK" {
            return Err(Error::Connection(format!("relay {addr}: refused")));
        }
        reader.get_ref().set_read_timeout(None).map_err(fail)?;
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("masterbus-relay-client".into())
            .spawn(move || pump(reader, tx))
            .map_err(fail)?;
        Ok(RelayTransport { rx, stream, addr })
    }

    /// The relay's address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

fn pump(mut reader: BufReader<TcpStream>, tx: Sender<Frame>) {
    while let Ok(f) = read_frame(&mut reader) {
        if tx.send(f).is_err() {
            return;
        }
    }
}

impl Transport for RelayTransport {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (Box::new(ClientRx(self.rx)), Box::new(ClientTx(self.stream)))
    }
}

struct ClientRx(Receiver<Frame>);

impl TransportRx for ClientRx {
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        match self.0.recv_timeout(timeout) {
            Ok(f) => Ok(Some(f)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(Error::Connection(
                "the daemon sharing the bus went away".into(),
            )),
        }
    }
}

struct ClientTx(TcpStream);

impl TransportTx for ClientTx {
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
        self.0
            .write_all(&encode(can_id, data))
            .map_err(|e| Error::Connection(format!("relay: {e}")))
    }
}

/// A relay announced in one of `dirs` that accepts us, if any.
pub fn find(dirs: &[PathBuf]) -> Option<RelayTransport> {
    for dir in dirs {
        let path = dir.join(RELAY_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let (Some(addr), Some(token)) = (
            v["address"].as_str().and_then(|a| a.parse().ok()),
            v["token"].as_str(),
        ) else {
            continue;
        };
        match RelayTransport::connect(addr, token) {
            Ok(t) => return Some(t),
            // Left behind by a daemon that has stopped.
            Err(e) => log::debug!("{}: {e}", path.display()),
        }
    }
    None
}

fn announce(dir: &Path, addr: SocketAddr, token: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(RELAY_FILE);
    let tmp = dir.join(format!(".{RELAY_FILE}.{}", std::process::id()));
    let body = serde_json::json!({
        "address": addr.to_string(),
        "token": token,
        "pid": std::process::id(),
    });
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(body.to_string().as_bytes())?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// 128 random bits, hex. The OS seeds `RandomState` from its own entropy.
fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn encode(id: u32, data: &[u8]) -> Vec<u8> {
    let len = data.len().min(8);
    let mut b = Vec::with_capacity(5 + len);
    b.extend_from_slice(&id.to_be_bytes());
    b.push(len as u8);
    b.extend_from_slice(&data[..len]);
    b
}

fn read_frame(r: &mut impl Read) -> std::io::Result<Frame> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = head[4] as usize;
    if len > 8 {
        return Err(std::io::Error::other("frame longer than 8 bytes"));
    }
    let mut data = vec![0u8; len];
    r.read_exact(&mut data)?;
    Ok((
        u32::from_be_bytes([head[0], head[1], head[2], head[3]]),
        data,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wire that records what is sent and plays back what is queued.
    struct Wire {
        incoming: Receiver<Frame>,
        sent: Sender<Frame>,
    }
    struct WireRx(Receiver<Frame>);
    struct WireTx(Sender<Frame>);
    impl Transport for Wire {
        fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
            (Box::new(WireRx(self.incoming)), Box::new(WireTx(self.sent)))
        }
    }
    impl TransportRx for WireRx {
        fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>> {
            Ok(self.0.recv_timeout(timeout).ok())
        }
    }
    impl TransportTx for WireTx {
        fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
            self.0.send((can_id, data.to_vec())).unwrap();
            Ok(())
        }
    }

    #[test]
    fn a_client_sees_the_wire_and_reaches_it() {
        let dir = std::env::temp_dir().join(format!("mb-relay-{}", std::process::id()));
        let (to_wire, incoming) = unbounded();
        let (sent, on_wire) = unbounded();
        let (own, _addr) =
            share(Box::new(Wire { incoming, sent }), "127.0.0.1:0", Some(&dir)).unwrap();
        let (mut own_rx, mut own_tx) = own.split();

        let client = find(std::slice::from_ref(&dir)).expect("announced relay");
        let (mut c_rx, mut c_tx) = Box::new(client).split();
        // Let the relay register the client before the wire talks.
        std::thread::sleep(Duration::from_millis(100));

        to_wire.send((0x0812_3456, vec![1, 2, 3])).unwrap();
        let want = Some((0x0812_3456, vec![1, 2, 3]));
        assert_eq!(own_rx.recv(Duration::from_secs(2)).unwrap(), want);
        assert_eq!(c_rx.recv(Duration::from_secs(2)).unwrap(), want);

        c_tx.send(0x1812_3456, &[7, 0]).unwrap();
        own_tx.send(0x0700_0001, &[]).unwrap();
        let mut got: Vec<Frame> = (0..2)
            .map(|_| on_wire.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        got.sort();
        assert_eq!(got, vec![(0x0700_0001, vec![]), (0x1812_3456, vec![7, 0])]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let dir = std::env::temp_dir().join(format!("mb-relay-tok-{}", std::process::id()));
        let (_to_wire, incoming) = unbounded();
        let (sent, _on_wire) = unbounded();
        let (_own, addr) =
            share(Box::new(Wire { incoming, sent }), "127.0.0.1:0", Some(&dir)).unwrap();
        assert!(RelayTransport::connect(addr, "guess").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_announcement_is_skipped() {
        let dir = std::env::temp_dir().join(format!("mb-relay-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Bind and drop: nothing listens there any more.
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        announce(&dir, addr, "x").unwrap();
        assert!(find(std::slice::from_ref(&dir)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
