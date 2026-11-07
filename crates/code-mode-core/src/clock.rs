use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Time source for cache TTL checks. Tests inject [`ManualClock`].
pub trait Clock: Send + Sync {
    fn unix_secs(&self) -> i64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_secs(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// Controllable clock. `secs` starts at 0.
#[derive(Debug, Default)]
pub struct ManualClock {
    secs: AtomicI64,
}

impl ManualClock {
    pub fn new(secs: i64) -> Self {
        Self {
            secs: AtomicI64::new(secs),
        }
    }

    pub fn set(&self, secs: i64) {
        self.secs.store(secs, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn unix_secs(&self) -> i64 {
        self.secs.load(Ordering::SeqCst)
    }
}
