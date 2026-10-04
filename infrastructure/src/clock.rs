//! The platform clock, behind the port.

use application::ports::Clock;
use std::time::SystemTime;

/// Reads the time from the platform.
///
/// The only implementation of [`Clock`] that does. Everything else in the
/// workspace takes the port, so a test supplies a moment it chose.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
