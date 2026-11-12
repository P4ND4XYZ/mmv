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
    /// One absolute deadline covers the entire stage, including partial I/O.
    pub async fn until<T>(
        self,
        future: impl std::future::Future<Output = std::io::Result<T>>,
    ) -> std::io::Result<T> {
        if self.expired(Instant::now()) {
            return Err(Self::timeout());
        }
        tokio::time::timeout_at(self.0.into(), future)
            .await
            .map_err(|_| Self::timeout())?
    }
    pub(super) fn timeout() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "IPC deadline exceeded")
    }
    /// Only endpoint/startup arbitration retries; never retry a sent request.
    pub(super) async fn pause(self) -> std::io::Result<()> {
        self.until(async {
            tokio::time::sleep(
                self.remaining(Instant::now())
                    .min(Duration::from_millis(10)),
            )
            .await;
            Ok(())
        })
        .await
    }
    #[cfg(test)]
    pub(super) fn after(duration: Duration) -> Self {
        Self(Instant::now() + duration)
    }
}
