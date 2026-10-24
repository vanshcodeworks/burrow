//! Restart backoff and the crash-loop breaker.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default cap on restarts before the crash-loop breaker trips.
pub const DEFAULT_MAX_FAILURES: u32 = 5;
/// Default window in which failures are counted.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
const DEFAULT_BASE_DELAY: Duration = Duration::from_millis(500);
const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(30);
/// A run at least this long resets the consecutive-failure counter.
const STABLE_UPTIME: Duration = Duration::from_secs(30);

/// What the supervisor should do after an unexpected exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartDecision {
    /// Wait this long, then restart.
    RestartAfter(Duration),
    /// Stop retrying and surface a crash-loop error.
    CrashLoop {
        /// Failures observed inside the window.
        failures: u32,
        /// The window used for the decision.
        window: Duration,
    },
}

/// Tracks failure timestamps and computes exponential backoff delays.
#[derive(Debug, Clone)]
pub struct RestartBackoff {
    max_failures: u32,
    window: Duration,
    base_delay: Duration,
    max_delay: Duration,
    failures: VecDeque<Instant>,
    consecutive: u32,
    last_started: Option<Instant>,
}

impl Default for RestartBackoff {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FAILURES, DEFAULT_WINDOW)
    }
}

impl RestartBackoff {
    /// Creates a tracker with an explicit crash-loop budget.
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures: max_failures.max(1),
            window,
            base_delay: DEFAULT_BASE_DELAY,
            max_delay: DEFAULT_MAX_DELAY,
            failures: VecDeque::new(),
            consecutive: 0,
            last_started: None,
        }
    }

    /// Overrides the first backoff delay (used by tests).
    pub fn with_delays(mut self, base: Duration, max: Duration) -> Self {
        self.base_delay = base;
        self.max_delay = max.max(base);
        self
    }

    /// Records a successful start.
    pub fn mark_started(&mut self) {
        self.last_started = Some(Instant::now());
    }

    /// Records an unexpected exit and returns the restart decision.
    ///
    /// A run that stayed up for [`STABLE_UPTIME`] resets the consecutive
    /// failure count, so a service that crashes once a day is not treated as a
    /// crash loop.
    pub fn on_failure(&mut self, uptime: Duration) -> RestartDecision {
        let now = Instant::now();
        if uptime >= STABLE_UPTIME {
            self.failures.clear();
            self.consecutive = 0;
        }
        self.failures
            .retain(|failure| now.duration_since(*failure) <= self.window);
        self.failures.push_back(now);

        self.consecutive = self.consecutive.saturating_add(1);

        if self.failures.len() as u32 >= self.max_failures {
            return RestartDecision::CrashLoop {
                failures: self.failures.len() as u32,
                window: self.window,
            };
        }

        let exponent = self.consecutive.saturating_sub(1).min(16);
        let delay = self
            .base_delay
            .saturating_mul(1u32 << exponent)
            .min(self.max_delay);
        RestartDecision::RestartAfter(delay)
    }

    /// Clears all recorded history. Used when the user explicitly restarts a
    /// service, which should get a fresh budget.
    pub fn reset(&mut self) {
        self.failures.clear();
        self.consecutive = 0;
        self.last_started = None;
    }

    /// Number of failures currently inside the window.
    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }

    /// When the service was last started, if it has been started.
    pub fn last_started(&self) -> Option<Instant> {
        self.last_started
    }
}

#[cfg(test)]
mod tests {
    use super::{RestartBackoff, RestartDecision};
    use std::time::Duration;

    #[test]
    fn delays_grow_exponentially_and_are_capped() {
        let mut backoff = RestartBackoff::new(10, Duration::from_secs(60))
            .with_delays(Duration::from_millis(100), Duration::from_millis(500));

        assert_eq!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(Duration::from_millis(100))
        );
        assert_eq!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(Duration::from_millis(200))
        );
        assert_eq!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(Duration::from_millis(400))
        );
        assert_eq!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(Duration::from_millis(500))
        );
    }

    #[test]
    fn breaker_trips_after_the_budget_is_used_inside_the_window() {
        let mut backoff = RestartBackoff::new(3, Duration::from_secs(60));
        assert!(matches!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(_)
        ));
        assert!(matches!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(_)
        ));
        assert_eq!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::CrashLoop {
                failures: 3,
                window: Duration::from_secs(60)
            }
        );
    }

    #[test]
    fn a_stable_run_resets_the_consecutive_count() {
        let mut backoff = RestartBackoff::new(5, Duration::from_secs(60))
            .with_delays(Duration::from_millis(100), Duration::from_millis(500));
        for _ in 0..4 {
            let _ = backoff.on_failure(Duration::ZERO);
        }
        // Four failures, then a long stable run: the counter resets even though
        // the old failures are still inside the window.
        assert_eq!(
            backoff.on_failure(Duration::from_secs(60)),
            RestartDecision::RestartAfter(Duration::from_millis(100))
        );
        assert_eq!(backoff.failure_count(), 1);
    }

    #[test]
    fn reset_clears_history() {
        let mut backoff = RestartBackoff::new(2, Duration::from_secs(60));
        let _ = backoff.on_failure(Duration::ZERO);
        backoff.reset();
        assert_eq!(backoff.failure_count(), 0);
        assert!(matches!(
            backoff.on_failure(Duration::ZERO),
            RestartDecision::RestartAfter(_)
        ));
    }
}
