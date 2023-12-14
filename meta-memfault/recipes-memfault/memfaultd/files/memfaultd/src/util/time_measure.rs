//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::time::{Duration, Instant};

/// A trait for measuring time.
///
/// This is mostly a way to mock std::time::Instant for testing.
pub trait TimeMeasure {
    fn now() -> Self;
    fn elapsed(&self) -> Duration;
    fn since(&self, other: &Self) -> Duration;
}

impl TimeMeasure for Instant {
    fn now() -> Self {
        Instant::now()
    }

    fn elapsed(&self) -> Duration {
        Self::now().since(self)
    }

    fn since(&self, other: &Self) -> Duration {
        Instant::duration_since(self, *other)
    }
}
