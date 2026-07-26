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

    /// Cancel all pending waits (on shutdown).
    pub fn cancel_all(&self) {
        let mut map = self.inner.lock().unwrap();
        for slot in map.values_mut() {
            slot.cancelled = true;
        }
        self.cv.notify_all();
    }
}
