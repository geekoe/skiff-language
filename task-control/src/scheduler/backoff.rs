//! Platform-level bounded retry backoff for infrastructure recovery.
//!
//! This is not a user-facing application-error retry policy. It paces the
//! at-least-once recovery path so unavailable Runtimes, store outages or
//! poisoned tasks never form a hot retry loop. The delay grows
//! exponentially per attempt generation, is capped at a platform maximum,
//! and carries a bounded jitter below a configured span.

use std::sync::Mutex;

use crate::model::DurableDuration;

/// Bounded jitter source. Tests inject fixed values; production uses an LCG.
pub trait Jitter: Send + Sync {
    /// Return a non-negative jitter value in `[0, span)` millis. `span <= 0`
    /// must return `0`.
    fn next_millis(&self, span: i64) -> i64;
}

/// Deterministic fixed jitter for tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedJitter(pub i64);

impl Jitter for FixedJitter {
    fn next_millis(&self, span: i64) -> i64 {
        if span <= 0 {
            0
        } else {
            self.0.clamp(0, span - 1)
        }
    }
}

/// Small deterministic LCG. Suitable for spreading retries, not for
/// cryptography; reproducibility from a seed is deliberate.
#[derive(Debug)]
pub struct LcgJitter {
    state: Mutex<u64>,
}

impl LcgJitter {
    pub fn new(seed: u64) -> Self {
        Self {
            state: Mutex::new(seed),
        }
    }
}

impl Jitter for LcgJitter {
    fn next_millis(&self, span: i64) -> i64 {
        if span <= 0 {
            return 0;
        }
        let mut state = self.state.lock().expect("lcg jitter lock");
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (*state % span as u64) as i64
    }
}

/// Bounded backoff policy. `delay(attempt_generation)` is
/// `min(base * 2^(generation - 1), max) + jitter`, with
/// `jitter < jitter_span`; the total upper bound is
/// `max + jitter_span - 1` millis.
pub struct RetryBackoffPolicy {
    pub base: DurableDuration,
    pub max: DurableDuration,
    pub jitter_span: DurableDuration,
    pub jitter: Box<dyn Jitter>,
}

const DEFAULT_BASE_MILLIS: i64 = 100;
const DEFAULT_MAX_MILLIS: i64 = 30_000;
const DEFAULT_JITTER_SPAN_MILLIS: i64 = 100;
const DEFAULT_JITTER_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

impl Default for RetryBackoffPolicy {
    fn default() -> Self {
        Self::with_jitter(
            DurableDuration::from_millis(DEFAULT_BASE_MILLIS),
            DurableDuration::from_millis(DEFAULT_MAX_MILLIS),
            DurableDuration::from_millis(DEFAULT_JITTER_SPAN_MILLIS),
            Box::new(LcgJitter::new(DEFAULT_JITTER_SEED)),
        )
        .expect("default backoff policy is valid")
    }
}

impl RetryBackoffPolicy {
    /// Build a policy with an explicit jitter source. `base` must be positive,
    /// `max >= base` and `jitter_span >= 0`.
    pub fn with_jitter(
        base: DurableDuration,
        max: DurableDuration,
        jitter_span: DurableDuration,
        jitter: Box<dyn Jitter>,
    ) -> Result<Self, String> {
        if base.millis() <= 0 {
            return Err("backoff base must be positive".to_string());
        }
        if max.millis() < base.millis() {
            return Err("backoff max must be >= base".to_string());
        }
        if jitter_span.millis() < 0 {
            return Err("backoff jitter span must be non-negative".to_string());
        }
        Ok(Self {
            base,
            max,
            jitter_span,
            jitter,
        })
    }

    /// Backoff millis for a given attempt generation.
    pub fn delay_millis(&self, attempt_generation: u64) -> i64 {
        let exponent = attempt_generation.saturating_sub(1).min(62);
        let doubled = self.base.millis().saturating_mul(1i64 << exponent);
        let capped = doubled.min(self.max.millis());
        capped.saturating_add(self.jitter.next_millis(self.jitter_span.millis()))
    }

    /// [`Self::delay_millis`] as a durable duration.
    pub fn delay(&self, attempt_generation: u64) -> DurableDuration {
        DurableDuration::from_millis(self.delay_millis(attempt_generation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_exponential_then_capped_and_jittered() {
        let policy = RetryBackoffPolicy {
            base: DurableDuration::from_millis(10),
            max: DurableDuration::from_millis(40),
            jitter_span: DurableDuration::from_millis(5),
            jitter: Box::new(FixedJitter(3)),
        };
        assert_eq!(policy.delay_millis(0), 13, "generation 0 falls back to 1");
        assert_eq!(policy.delay_millis(1), 13);
        assert_eq!(policy.delay_millis(2), 23);
        assert_eq!(policy.delay_millis(3), 43);
        assert_eq!(policy.delay_millis(4), 43, "exponential growth caps");
        assert_eq!(policy.delay_millis(100), 43, "upper bound holds");
        assert!(policy.delay_millis(1) <= 40 + 5 - 1);
    }

    #[test]
    fn jitter_is_bounded_and_fixed_jitter_clamps() {
        assert_eq!(FixedJitter(9).next_millis(5), 4);
        assert_eq!(FixedJitter(9).next_millis(0), 0);
        assert_eq!(FixedJitter(-2).next_millis(5), 0);
        let policy = RetryBackoffPolicy::with_jitter(
            DurableDuration::from_millis(1),
            DurableDuration::from_millis(10),
            DurableDuration::from_millis(7),
            Box::new(LcgJitter::new(42)),
        )
        .expect("policy");
        for generation in 1..=20u64 {
            let capped = (1i64 << generation.saturating_sub(1)).min(10);
            let jitter = policy.delay_millis(generation) - capped;
            assert!(
                (0..7).contains(&jitter),
                "jitter must stay in [0, span) for generation {generation}"
            );
        }
    }

    #[test]
    fn policy_validation_rejects_illegal_bounds() {
        assert!(RetryBackoffPolicy::with_jitter(
            DurableDuration::from_millis(0),
            DurableDuration::from_millis(1),
            DurableDuration::from_millis(0),
            Box::new(FixedJitter(0)),
        )
        .is_err());
        assert!(RetryBackoffPolicy::with_jitter(
            DurableDuration::from_millis(2),
            DurableDuration::from_millis(1),
            DurableDuration::from_millis(0),
            Box::new(FixedJitter(0)),
        )
        .is_err());
        assert!(RetryBackoffPolicy::with_jitter(
            DurableDuration::from_millis(1),
            DurableDuration::from_millis(2),
            DurableDuration::from_millis(-1),
            Box::new(FixedJitter(0)),
        )
        .is_err());
    }
}
