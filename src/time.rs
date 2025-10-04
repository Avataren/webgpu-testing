#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
use core::ops::Sub;
#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instant {
    millis: f64,
}

#[cfg(target_arch = "wasm32")]
impl Instant {
    pub fn now() -> Self {
        Self {
            millis: performance_now(),
        }
    }

    pub fn duration_since(&self, earlier: Instant) -> Duration {
        Duration::from_secs_f64((self.millis - earlier.millis).max(0.0) / 1000.0)
    }

    pub fn elapsed(&self) -> Duration {
        Self::now() - *self
    }
}

#[cfg(target_arch = "wasm32")]
impl Sub<Instant> for Instant {
    type Output = Duration;

    fn sub(self, rhs: Instant) -> Duration {
        Duration::from_secs_f64((self.millis - rhs.millis).max(0.0) / 1000.0)
    }
}

#[cfg(target_arch = "wasm32")]
fn performance_now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|perf| perf.now())
        .unwrap_or(0.0)
}
