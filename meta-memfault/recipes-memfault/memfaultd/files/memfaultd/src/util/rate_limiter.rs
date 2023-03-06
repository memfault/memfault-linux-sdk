//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Support for rate limiting some execution paths.
use eyre::Result;
use ratelimit::Ratelimiter;

/// A rate limiter which keeps track of how many calls were rate limited.
/// You can also provide a `info: I` parameter with each call. The latest one
/// will be passed to the runner when not rate limited anymore.
pub struct RateLimiter<I> {
    rate_limiter: Ratelimiter,
    limited_calls: Option<RateLimitedCalls<I>>,
}

pub struct RateLimitedCalls<I> {
    pub count: usize,
    pub latest_call: I,
}

impl<I> RateLimiter<I> {
    /// Create a new rate limiter with given capacity, quantum and rate (see ratelimit::Ratelimiter).
    pub fn new(capacity: u64, quantum: u64, rate: u64) -> Self {
        Self {
            rate_limiter: Ratelimiter::new(capacity, quantum, rate),
            limited_calls: None,
        }
    }

    /// Run the provided work function if the rate limitings limits have not been reached.
    pub fn run_within_limits<W>(&mut self, info: I, work: W) -> Result<()>
    where
        W: FnOnce(Option<RateLimitedCalls<I>>) -> Result<()>,
    {
        if self.rate_limiter.try_wait().is_ok() {
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
