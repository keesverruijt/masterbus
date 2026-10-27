//! Request/response correlation.
//!
//! MasterBus responses carry no correlation token, so each in-flight request is
//! keyed by a string (see [`crate::protocol::waiter_key_for_frame`] and the
//! value keys built by the scheduler). The reader thread deposits the matching
//! payload; the requesting thread blocks until it arrives or times out.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

struct Slot {
    data: Option<Vec<u8>>,
    cancelled: bool,
}

/// A string-keyed request waiter shared between the reader and the scheduler.
#[derive(Default)]
pub struct Waiter {
    inner: Mutex<HashMap<String, Slot>>,
    cv: Condvar,
}

impl Waiter {
    /// Create an empty waiter.
    pub fn new() -> Self {
        Waiter {
            inner: Mutex::new(HashMap::new()),
            cv: Condvar::new(),
        }
    }

    /// Register interest in `key` **before** sending the request.
    pub fn register(&self, key: &str) {
        self.inner.lock().unwrap().insert(
            key.to_string(),
            Slot {
                data: None,
                cancelled: false,
            },
        );
    }

    /// Deliver a response payload to a waiter registered for `key`. No-op if no
    /// one is waiting (unsolicited frame).
    pub fn deliver(&self, key: &str, data: Vec<u8>) {
        let mut map = self.inner.lock().unwrap();
        if let Some(slot) = map.get_mut(key) {
            slot.data = Some(data);
            self.cv.notify_all();
        }
    }

    /// Block until a response for `key` arrives or `timeout` elapses. Removes the
    /// key on return. `None` on timeout/cancellation.
    pub fn wait(&self, key: &str, timeout: Duration) -> Option<Vec<u8>> {
        let mut map = self.inner.lock().unwrap();
        let (m, _) = self
            .cv
            .wait_timeout_while(map, timeout, |m| {
                m.get(key)
                    .map(|s| s.data.is_none() && !s.cancelled)
                    .unwrap_or(false)
            })
            .unwrap();
        map = m;
        map.remove(key)
            .and_then(|s| if s.cancelled { None } else { s.data })
    }

    /// Like [`Waiter::wait`], but on timeout `key` stays registered, under
    /// the same lock, so a reply that arrives later is kept rather than
    /// dropped. Collect it with [`Waiter::wait`].
    pub fn wait_or_keep(&self, key: &str, timeout: Duration) -> Option<Vec<u8>> {
        let map = self.inner.lock().unwrap();
        let (mut map, _) = self
            .cv
            .wait_timeout_while(map, timeout, |m| {
                m.get(key)
                    .map(|s| s.data.is_none() && !s.cancelled)
                    .unwrap_or(false)
            })
            .unwrap();
        match map.get(key) {
            Some(s) if s.data.is_none() && !s.cancelled => None,
            _ => map
                .remove(key)
                .and_then(|s| if s.cancelled { None } else { s.data }),
        }
    }

    /// Block until one of `keys` has a response (or is cancelled), or until
    /// `timeout` elapses. Returns the position in `keys` of one that is ready,
    /// leaving it registered for [`Waiter::wait`] to collect; `None` on timeout.
    pub fn wait_any(&self, keys: &[&str], timeout: Duration) -> Option<usize> {
        let ready = |m: &HashMap<String, Slot>| {
            keys.iter().position(|k| {
                m.get(*k)
                    .map(|s| s.data.is_some() || s.cancelled)
                    .unwrap_or(true)
            })
        };
        let map = self.inner.lock().unwrap();
        let (map, _) = self
            .cv
            .wait_timeout_while(map, timeout, |m| ready(m).is_none())
            .unwrap();
        ready(&map)
    }

    /// Cancel all pending waits (on shutdown).
    pub fn cancel_all(&self) {
        let mut map = self.inner.lock().unwrap();
        for slot in map.values_mut() {
            slot.cancelled = true;
        }
        self.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply after `wait_or_keep` timed out is kept for a later `wait`;
    /// after plain `wait` times out it is dropped.
    #[test]
    fn a_kept_key_takes_a_late_reply() {
        let w = Waiter::new();
        w.register("k");
        assert_eq!(w.wait_or_keep("k", Duration::from_millis(1)), None);
        w.deliver("k", vec![1]);
        assert_eq!(w.wait("k", Duration::from_millis(1)), Some(vec![1]));

        w.register("d");
        assert_eq!(w.wait("d", Duration::from_millis(1)), None);
        w.deliver("d", vec![2]);
        assert_eq!(w.wait("d", Duration::from_millis(1)), None);
    }
}
