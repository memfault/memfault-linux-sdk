//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Support for rate limiting some execution paths.
use std::num::NonZeroU32;

use eyre::Result;
use governor::{
    clock,
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovRateLimiter,
};

/// A rate limiter which keeps track of how many calls were rate limited.
/// You can also provide a `info: I` parameter with each call. The latest one
/// will be passed to the runner when not rate limited anymore.
pub struct RateLimiter<I, Clock: clock::Clock = clock::DefaultClock> {
    rate_limiter: GovRateLimiter<NotKeyed, InMemoryState, Clock, NoOpMiddleware<Clock::Instant>>,
    limited_calls: Option<RateLimitedCalls<I>>,
}

pub struct RateLimitedCalls<I> {
    pub count: usize,
    pub latest_call: I,
}

impl<I> RateLimiter<I, clock::MonotonicClock> {
    /// Create a new rate limiter with given capacity, quantum and rate (see ratelimit::Ratelimiter).
    pub fn new(capacity_per_minute: NonZeroU32) -> Self {
        Self {
            rate_limiter: GovRateLimiter::direct(Quota::per_minute(capacity_per_minute)),
            limited_calls: None,
        }
    }
}

impl<I, C: clock::Clock> RateLimiter<I, C> {
    /// Run the provided work function if the rate limitings limits have not been reached.
    pub fn run_within_limits<W>(&mut self, info: I, work: W) -> Result<()>
    where
        W: FnOnce(Option<RateLimitedCalls<I>>) -> Result<()>,
    {
        if self.rate_limiter.check().is_ok() {
            work(self.limited_calls.take())
        } else {
            self.limited_calls = Some(match self.limited_calls.take() {
                None => RateLimitedCalls {
                    count: 1,
                    latest_call: info,
                },
                Some(l) => RateLimitedCalls {
                    count: l.count + 1,
                    latest_call: info,
                },
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU32, time::Duration};

    use governor::clock;
    use governor::clock::FakeRelativeClock;
    use rstest::fixture;
    use rstest::rstest;

    use super::RateLimiter;

    #[rstest]
    fn test_sustained_100_per_minute(mut rl: RLFixture) {
        for _ in 0..20 {
            rl.assert_wait_and_grab_tokens(15_000, 25);
        }
    }

    #[rstest]
    fn test_bursty_start(mut rl: RLFixture) {
        rl.assert_wait_and_grab_tokens(0, 100);
        rl.assert_empty();
        rl.assert_wait_and_grab_tokens(15_000, 25);
        rl.assert_empty();
    }

    #[rstest]
    fn test_reject_burst(mut rl: RLFixture) {
        rl.assert_wait_and_grab_tokens(200_000, 100);
        rl.assert_empty();
        rl.assert_wait_and_grab_tokens(1000, 1);
    }

    #[fixture(limit = 100)]
    fn rl(limit: u32) -> RLFixture {
        let clock = FakeRelativeClock::default();
        RLFixture {
            rl: RateLimiter::new_with_clock(NonZeroU32::new(limit).unwrap(), &clock),
            clock,
        }
    }

    struct RLFixture {
        rl: RateLimiter<(), FakeRelativeClock>,
        clock: FakeRelativeClock,
    }

    impl RLFixture {
        pub fn assert_wait_and_grab_tokens(&mut self, sleep_ms: u64, count_tokens: u64) {
            let grabbed = self.wait_and_grab_tokens(sleep_ms, count_tokens);
            assert!(
                count_tokens == grabbed,
                "Expected to grab {count_tokens} but only {grabbed} available."
            );
        }

        pub fn assert_empty(&mut self) {
            assert!(self.grab_tokens(1) == 0, "Rate limiter is not empty");
        }

        pub fn wait_and_grab_tokens(&mut self, sleep_ms: u64, count_tokens: u64) -> u64 {
            self.clock.advance(Duration::from_millis(sleep_ms));
            self.grab_tokens(count_tokens)
        }

        pub fn grab_tokens(&mut self, c: u64) -> u64 {
            for i in 0..c {
                let work_done = &mut false;
                let _result = self.rl.run_within_limits((), |_info| {
                    *work_done = true;
                    Ok(())
                });
                if !*work_done {
                    return i;
                }
            }
            c
        }
    }

    impl<I, C: clock::Clock> RateLimiter<I, C> {
        #[cfg(test)]
        pub fn new_with_clock(capacity: NonZeroU32, clock: &C) -> Self {
            use governor::Quota;
            use governor::RateLimiter as GovRateLimiter;

            Self {
                rate_limiter: GovRateLimiter::direct_with_clock(Quota::per_minute(capacity), clock),
                limited_calls: None,
            }
        }
    }
}
