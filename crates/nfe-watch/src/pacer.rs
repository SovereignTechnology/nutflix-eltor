//! Keep a viewer's pulls under every hop's per-round byte quota.
//!
//! El Tor Foundation closes a circuit the moment a hop's quota for the current round is exceeded, so the viewer
//! must never pull faster than the path's payload budget (`nfe_eltor::price::payload_budget_per_round`). The pacer
//! counts rounds of 60 s from when it starts (the circuit's activation, as close as the viewer can tell) and makes a
//! fetch wait for the next round when the current one is spent. The budget already keeps a safety margin, which
//! also absorbs the difference between our round boundaries and the relays'.

use std::time::{Duration, Instant};

pub const ROUND: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PaceError {
    #[error("{0} bytes can never fit in one round's budget of {1}")]
    TooBig(u64, u64),
}

/// Time source, injectable for tests.
pub trait Clock: Send {
    fn now(&self) -> Instant;
    fn sleep(&mut self, d: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

pub struct Pacer<C: Clock = SystemClock> {
    /// Payload bytes per round; `None` = no hop sets a quota.
    budget: Option<u64>,
    start: Instant,
    round: u64,
    used: u64,
    clock: C,
}

impl Pacer<SystemClock> {
    pub fn new(budget: Option<u64>) -> Self {
        Self::with_clock(budget, SystemClock)
    }
}

impl<C: Clock> Pacer<C> {
    pub fn with_clock(budget: Option<u64>, clock: C) -> Self {
        Self { budget, start: clock.now(), round: 0, used: 0, clock }
    }

    fn current_round(&self) -> u64 {
        let elapsed = self.clock.now().saturating_duration_since(self.start);
        (elapsed.as_millis() / ROUND.as_millis()) as u64
    }

    /// Reserve `bytes` before fetching them, waiting for later rounds as needed.
    pub fn reserve(&mut self, bytes: u64) -> Result<(), PaceError> {
        let Some(budget) = self.budget else { return Ok(()) };
        if bytes > budget {
            return Err(PaceError::TooBig(bytes, budget));
        }
        loop {
            let now_round = self.current_round();
            if now_round != self.round {
                self.round = now_round;
                self.used = 0;
            }
            if self.used + bytes <= budget {
                self.used += bytes;
                return Ok(());
            }
            let next = self.start + ROUND * u32::try_from(self.round + 1).unwrap_or(u32::MAX);
            let wait = next.saturating_duration_since(self.clock.now());
            self.clock.sleep(wait.max(Duration::from_millis(1)));
        }
    }

    /// Give back part of a reservation that turned out larger than what was fetched (same round only).
    pub fn refund(&mut self, bytes: u64) {
        if self.current_round() == self.round {
            self.used = self.used.saturating_sub(bytes);
        }
    }

    pub fn budget(&self) -> Option<u64> {
        self.budget
    }

    pub fn used_this_round(&self) -> u64 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClock {
        t: Instant,
        slept: Duration,
    }
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.t
        }
        fn sleep(&mut self, d: Duration) {
            self.t += d;
            self.slept += d;
        }
    }

    fn pacer(budget: Option<u64>) -> Pacer<FakeClock> {
        Pacer::with_clock(budget, FakeClock { t: Instant::now(), slept: Duration::ZERO })
    }

    #[test]
    fn waits_for_the_next_round_when_spent() {
        let mut p = pacer(Some(100));
        p.reserve(60).unwrap();
        p.reserve(40).unwrap();
        assert_eq!(p.clock.slept, Duration::ZERO);
        p.reserve(1).unwrap();
        assert_eq!(p.clock.slept, ROUND, "waited exactly to the next round boundary");
        assert_eq!(p.used_this_round(), 1);
        p.clock.t += Duration::from_secs(30);
        p.reserve(99).unwrap();
        p.reserve(50).unwrap();
        assert_eq!(p.clock.slept, ROUND + Duration::from_secs(30));
    }

    #[test]
    fn refund_and_limits() {
        let mut p = pacer(Some(100));
        p.reserve(100).unwrap();
        p.refund(70);
        p.reserve(70).unwrap();
        assert_eq!(p.clock.slept, Duration::ZERO);
        assert_eq!(p.reserve(101), Err(PaceError::TooBig(101, 100)));
        let mut unlimited = pacer(None);
        unlimited.reserve(u64::MAX).unwrap();
    }
}
