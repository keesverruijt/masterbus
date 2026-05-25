//! Linux SocketCAN transport.

use std::time::Duration;

use socketcan::{CanFrame, CanSocket, EmbeddedFrame, ExtendedId, Frame, Socket, SocketOptions};

use super::{Transport, TransportRx, TransportTx};
use crate::error::{Error, Result};

/// A SocketCAN transport using separate read and write sockets.
pub struct SocketCanTransport {
    read: CanSocket,
    write: CanSocket,
}

impl SocketCanTransport {
    /// Open the given CAN interface (e.g. `"can0"`, `"can-masterbus"`).
    pub fn open(interface: &str) -> Result<Self> {
        let read = CanSocket::open(interface)
            .map_err(|e| Error::Connection(format!("{interface}: {e}")))?;
        let write = CanSocket::open(interface)
            .map_err(|e| Error::Connection(format!("{interface}: {e}")))?;
        // Don't hear our own transmissions as if they were device replies.
        let _ = read.set_loopback(false);
        let _ = write.set_loopback(false);
        Ok(SocketCanTransport { read, write })
    }
}

impl Transport for SocketCanTransport {
    fn split(self: Box<Self>) -> (Box<dyn TransportRx>, Box<dyn TransportTx>) {
        (Box::new(Rx { sock: self.read }), Box::new(Tx { sock: self.write }))
    }
}

struct Rx {
    sock: CanSocket,
}

impl TransportRx for Rx {
    fn recv(&mut self, timeout: Duration) -> Result<Option<(u32, Vec<u8>)>> {
        self.sock.set_read_timeout(timeout).ok();
        match self.sock.read_frame() {
            Ok(frame) => Ok(Some((frame.raw_id(), frame.data().to_vec()))),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                Ok(None)
            }
            Err(e) => Err(Error::Connection(e.to_string())),
        }
    }
}

struct Tx {
    sock: CanSocket,
}

impl TransportTx for Tx {
    fn send(&mut self, can_id: u32, data: &[u8]) -> Result<()> {
        let eid = ExtendedId::new(can_id).ok_or_else(|| Error::Protocol("invalid CAN id".into()))?;
        let frame = CanFrame::new(eid, data).ok_or_else(|| Error::Protocol("invalid frame".into()))?;
        self.sock.write_frame(&frame).map_err(|e| Error::Connection(e.to_string()))?;
        Ok(())
    }
}
