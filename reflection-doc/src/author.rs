use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use crate::identity::PublicKey;

#[derive(Debug)]
pub struct Author {
    public_key: PublicKey,
    last_seen: Mutex<Option<DateTime<Utc>>>,
    is_online: AtomicBool,
    is_this_device: bool,
    last_cursor_update: Mutex<Option<SystemTime>>,
}

impl Clone for Author {
    fn clone(&self) -> Self {
        Self {
            public_key: self.public_key.clone(),
            last_seen: Mutex::new(*self.last_seen.lock().unwrap()),
            is_online: AtomicBool::new(self.is_online.load(Ordering::SeqCst)),
            is_this_device: self.is_this_device,
            last_cursor_update: Mutex::new(*self.last_cursor_update.lock().unwrap()),
        }
    }
}

impl Author {
    pub(crate) fn new(public_key: &PublicKey) -> Self {
        Self {
            public_key: public_key.clone(),
            last_seen: Mutex::new(None),
            is_online: AtomicBool::new(true),
            is_this_device: false,
            last_cursor_update: Mutex::new(None),
        }
    }

    pub(crate) fn with_state(public_key: &PublicKey, last_seen: Option<&DateTime<Utc>>) -> Self {
        Self {
            public_key: public_key.clone(),
            last_seen: Mutex::new(last_seen.copied()),
            is_online: AtomicBool::new(last_seen.is_none()),
            is_this_device: false,
            last_cursor_update: Mutex::new(None),
        }
    }

    pub(crate) fn for_this_device(
        public_key: &PublicKey,
        last_seen: Option<&DateTime<Utc>>,
    ) -> Self {
        Self {
            public_key: public_key.clone(),
            last_seen: Mutex::new(last_seen.copied()),
            is_online: AtomicBool::new(true),
            is_this_device: true,
            last_cursor_update: Mutex::new(None),
        }
    }

    pub(crate) fn public_key(&self) -> PublicKey {
        self.public_key.clone()
    }

    pub(crate) fn set_online(&self, is_online: bool) {
        let was_online = self.is_online.swap(is_online, Ordering::SeqCst);
        if !is_online && was_online {
            *self.last_seen.lock().unwrap() = Some(Utc::now());
        }
    }

    pub(crate) fn is_new_cursor_position(&self, timestamp: SystemTime) -> bool {
        let mut last_cursor_update = self.last_cursor_update.lock().unwrap();

        if last_cursor_update.is_none() || timestamp >= last_cursor_update.unwrap() {
            *last_cursor_update = Some(timestamp);
            true
        } else {
            false
        }
    }
}
