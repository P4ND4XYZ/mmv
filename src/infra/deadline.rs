use std::time::{Duration, Instant};

/// Immutable per-stage deadlines: retries never reset a stage's clock.
#[derive(Clone, Copy, Debug)]
pub struct Deadline(Instant);
impl Deadline {
    pub fn connection(start: Instant) -> Self {
        Self(start + Duration::from_secs(5))
    }
    pub fn startup(start: Instant) -> Self {
        Self(start + Duration::from_secs(30))
    }
    // Also bounds replies for operations which do not need a document read.
    pub fn response(start: Instant) -> Self {
        Self(start + Duration::from_secs(30))
    }
    pub fn remaining(self, now: Instant) -> Duration {
        self.0.saturating_duration_since(now)
    }
    pub fn expired(self, now: Instant) -> bool {
        now >= self.0
    }
    #[cfg(test)]
    pub(super) fn after(duration: Duration) -> Self {
        Self(Instant::now() + duration)
    }
}
