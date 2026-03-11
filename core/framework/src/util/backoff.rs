//! Exponential backoff retry strategy
//!
//! Provides configurable exponential backoff for connection retries and other
//! network operations that may fail transiently.
//!
//! # Features
//!
//! - Configurable initial delay, maximum delay, and retry limit
//! - Optional jitter (±30%) to prevent thundering herd
//! - Optional total duration limit (time-boxed retries)
//! - Custom multiplier support (default: 2.0)
//! - Iterator-based API for composability with async code
//!
//! # Example
//!
//! ```rust
//! use std::time::Duration;
//! use actr_framework::ExponentialBackoff;
//!
//! // Basic usage — 5 retries with jitter
//! let backoff = ExponentialBackoff::builder()
//!     .initial_delay(Duration::from_millis(100))
//!     .max_delay(Duration::from_secs(30))
//!     .max_retries(5)
//!     .with_jitter()
//!     .build();
//!
//! for delay in backoff {
//!     println!("waiting {:?}", delay);
//! }
//! ```

use std::time::{Duration, Instant};

/// Exponential backoff iterator
///
/// Generates increasing delays using the exponential backoff algorithm.
///
/// Use [`BackoffBuilder`] to construct instances.
///
/// # Algorithm
///
/// ```text
/// delay(n) = min(initial * multiplier^n, max_delay) ± jitter
/// ```
///
/// The iterator yields `Duration` values and terminates when:
/// - `max_retries` is reached, **or**
/// - `max_total_duration` is exceeded
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    /// Current delay duration for the *next* yield
    current: Duration,
    /// Maximum delay duration (cap)
    max: Duration,
    /// Multiplier for exponential growth (default: 2.0)
    multiplier: f64,
    /// Current retry count
    retries: u32,
    /// Maximum number of retries (`None` = unlimited)
    max_retries: Option<u32>,
    /// Whether to add random jitter (±30%) to each delay
    jitter: bool,
    /// Optional total duration limit (across all retries)
    max_total_duration: Option<Duration>,
    /// Start time for tracking total duration (lazily initialized)
    start_time: Option<Instant>,
}

impl ExponentialBackoff {
    /// Create new exponential backoff (simple constructor, backward compatible)
    ///
    /// For more options use [`BackoffBuilder`] via [`ExponentialBackoff::builder()`].
    pub fn new(initial: Duration, max: Duration, max_retries: Option<u32>) -> Self {
        Self {
            current: initial,
            max,
            multiplier: 2.0,
            retries: 0,
            max_retries,
            jitter: false,
            max_total_duration: None,
            start_time: None,
        }
    }

    /// Create backoff with custom multiplier (backward compatible)
    pub fn with_multiplier(
        initial: Duration,
        max: Duration,
        max_retries: Option<u32>,
        multiplier: f64,
    ) -> Self {
        Self {
            multiplier,
            ..Self::new(initial, max, max_retries)
        }
    }

    /// Create backoff with total duration limit (backward compatible)
    pub fn with_total_duration(
        initial: Duration,
        max: Duration,
        max_retries: Option<u32>,
        max_total_duration: Duration,
    ) -> Self {
        Self {
            max_total_duration: Some(max_total_duration),
            start_time: Some(Instant::now()),
            ..Self::new(initial, max, max_retries)
        }
    }

    /// Start building an `ExponentialBackoff` with the builder pattern
    ///
    /// # Example
    /// ```rust
    /// use std::time::Duration;
    /// use actr_framework::ExponentialBackoff;
    ///
    /// let backoff = ExponentialBackoff::builder()
    ///     .initial_delay(Duration::from_secs(1))
    ///     .max_delay(Duration::from_secs(60))
    ///     .max_retries(10)
    ///     .multiplier(1.5)
    ///     .with_jitter()
    ///     .build();
    /// ```
    pub fn builder() -> BackoffBuilder {
        BackoffBuilder::default()
    }

    /// Get current retry count
    pub fn retry_count(&self) -> u32 {
        self.retries
    }

    /// Reset backoff to initial state
    pub fn reset(&mut self, initial: Duration) {
        self.retries = 0;
        self.current = initial;
        if self.max_total_duration.is_some() {
            self.start_time = Some(Instant::now());
        }
    }

    /// Check if total duration has been exceeded
    fn is_duration_exceeded(&self) -> bool {
        if let (Some(max_duration), Some(start)) = (self.max_total_duration, self.start_time) {
            start.elapsed() > max_duration
        } else {
            false
        }
    }

    /// Apply jitter to a duration (±30%)
    fn apply_jitter(duration: Duration) -> Duration {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};

        // Use RandomState for lightweight pseudo-random without pulling in `rand`
        let random = RandomState::new().build_hasher().finish();
        // Map to range [-0.3, +0.3]
        let factor = ((random % 601) as f64 / 1000.0) - 0.3; // -0.3 to +0.3
        let jittered_millis = duration.as_millis() as f64 * (1.0 + factor);
        Duration::from_millis(jittered_millis.max(1.0) as u64)
    }
}

impl Iterator for ExponentialBackoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        // Initialize start_time on first call if max_total_duration is set
        if self.max_total_duration.is_some() && self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Check total duration limit first
        if self.is_duration_exceeded() {
            return None;
        }

        // Check if max retries reached
        if let Some(max_retries) = self.max_retries {
            if self.retries >= max_retries {
                return None;
            }
        }

        // Get current delay
        let delay = self.current;

        // Calculate next delay (exponential growth), capped at max
        let next_millis = (self.current.as_millis() as f64 * self.multiplier) as u64;
        let next_duration = Duration::from_millis(next_millis);
        self.current = next_duration.min(self.max);

        self.retries += 1;

        // Apply jitter if enabled
        if self.jitter {
            Some(Self::apply_jitter(delay))
        } else {
            Some(delay)
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Builder
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Builder for [`ExponentialBackoff`]
///
/// Provides a fluent API for constructing backoff iterators.
#[derive(Debug, Clone)]
pub struct BackoffBuilder {
    initial_delay: Duration,
    max_delay: Duration,
    max_retries: Option<u32>,
    multiplier: f64,
    jitter: bool,
    max_total_duration: Option<Duration>,
}

impl Default for BackoffBuilder {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_retries: None,
            multiplier: 2.0,
            jitter: false,
            max_total_duration: None,
        }
    }
}

impl BackoffBuilder {
    /// Set initial delay duration
    pub fn initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set maximum delay duration
    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Set maximum number of retries (`None` = unlimited)
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = Some(retries);
        self
    }

    /// Set multiplier for exponential growth (default: 2.0)
    pub fn multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// Enable jitter (±30% randomization on each delay)
    pub fn with_jitter(mut self) -> Self {
        self.jitter = true;
        self
    }

    /// Set total duration limit for all retries combined
    pub fn max_total_duration(mut self, duration: Duration) -> Self {
        self.max_total_duration = Some(duration);
        self
    }

    /// Build the [`ExponentialBackoff`] iterator
    pub fn build(self) -> ExponentialBackoff {
        ExponentialBackoff {
            current: self.initial_delay,
            max: self.max_delay,
            multiplier: self.multiplier,
            retries: 0,
            max_retries: self.max_retries,
            jitter: self.jitter,
            max_total_duration: self.max_total_duration,
            start_time: if self.max_total_duration.is_some() {
                Some(Instant::now())
            } else {
                None
            },
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_backoff() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(2), Some(4));

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(400)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(800)));
        assert_eq!(backoff.next(), None); // Exceeded max_retries
    }

    #[test]
    fn test_capped_backoff() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_millis(500),
            Some(5),
        );

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(400)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(500))); // Capped
        assert_eq!(backoff.next(), Some(Duration::from_millis(500))); // Still capped
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_unlimited_retries() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(50), Duration::from_secs(10), None);

        for i in 0..20 {
            let delay = backoff.next();
            assert!(delay.is_some(), "Retry {i} should succeed");
        }
    }

    #[test]
    fn test_custom_multiplier() {
        let mut backoff = ExponentialBackoff::with_multiplier(
            Duration::from_millis(100),
            Duration::from_secs(10),
            Some(3),
            1.5,
        );

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(150)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(225)));
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_retry_count() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(1), None);

        assert_eq!(backoff.retry_count(), 0);
        backoff.next();
        assert_eq!(backoff.retry_count(), 1);
        backoff.next();
        assert_eq!(backoff.retry_count(), 2);
    }

    #[test]
    fn test_total_duration_limit() {
        let backoff = ExponentialBackoff::with_total_duration(
            Duration::from_millis(10),
            Duration::from_millis(100),
            None,
            Duration::from_millis(50), // Very short total duration
        );

        let delays: Vec<_> = backoff.collect();
        // Should get at least 1 delay but not too many (time-limited)
        assert!(!delays.is_empty());
    }

    #[test]
    fn test_builder_basic() {
        let mut backoff = ExponentialBackoff::builder()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(2))
            .max_retries(4)
            .build();

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(400)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(800)));
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_builder_with_jitter() {
        let mut backoff = ExponentialBackoff::builder()
            .initial_delay(Duration::from_millis(1000))
            .max_delay(Duration::from_secs(60))
            .max_retries(3)
            .with_jitter()
            .build();

        let d1 = backoff.next().unwrap();
        let d2 = backoff.next().unwrap();
        let d3 = backoff.next().unwrap();

        // With jitter (±30%), 1000ms should be in range [700, 1300]
        assert!(
            d1.as_millis() >= 700 && d1.as_millis() <= 1300,
            "delay 1 = {:?}, expected 700-1300ms",
            d1
        );
        // 2000ms should be in range [1400, 2600]
        assert!(
            d2.as_millis() >= 1400 && d2.as_millis() <= 2600,
            "delay 2 = {:?}, expected 1400-2600ms",
            d2
        );
        // 4000ms should be in range [2800, 5200]
        assert!(
            d3.as_millis() >= 2800 && d3.as_millis() <= 5200,
            "delay 3 = {:?}, expected 2800-5200ms",
            d3
        );

        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_builder_with_custom_multiplier() {
        let mut backoff = ExponentialBackoff::builder()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(10))
            .max_retries(3)
            .multiplier(3.0)
            .build();

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(300)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(900)));
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn test_reset() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_millis(100), Duration::from_secs(2), Some(2));

        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
        assert_eq!(backoff.next(), Some(Duration::from_millis(200)));
        assert_eq!(backoff.next(), None);

        backoff.reset(Duration::from_millis(100));
        assert_eq!(backoff.retry_count(), 0);
        assert_eq!(backoff.next(), Some(Duration::from_millis(100)));
    }

    #[test]
    fn test_jitter_distribution() {
        // Verify jitter produces varied results
        let results: Vec<Duration> = (0..100)
            .map(|_| ExponentialBackoff::apply_jitter(Duration::from_millis(1000)))
            .collect();

        let min = results.iter().min().unwrap().as_millis();
        let max = results.iter().max().unwrap().as_millis();

        // Should have spread within ±30% range
        assert!(min < 1000, "min={min}ms, should be < 1000ms for jitter");
        assert!(max > 1000, "max={max}ms, should be > 1000ms for jitter");
    }
}
