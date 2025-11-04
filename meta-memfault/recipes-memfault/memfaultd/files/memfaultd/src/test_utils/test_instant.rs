//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{cell::RefCell, ops::Add, ops::Sub, time::Duration};

use crate::util::time_measure::TimeMeasure;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TestInstant {
    t0: Duration,
}

thread_local! {
    static TIME: RefCell<Duration>  = const { RefCell::new(Duration::from_secs(0)) };
}

impl TestInstant {
    pub fn now() -> Self {
        let mut now = Duration::from_secs(0);
        TIME.with(|t| now = *t.borrow());
        TestInstant { t0: now }
    }

    pub fn from(d: Duration) -> Self {
        TestInstant { t0: d }
    }

    pub fn sleep(d: Duration) {
        TIME.with(|t| {
            let new_time = t.borrow().saturating_add(d);
            *t.borrow_mut() = new_time;
        })
    }
}

impl TimeMeasure for TestInstant {
    fn now() -> Self {
        Self::now()
    }
    fn elapsed(&self) -> Duration {
        Self::now().t0 - self.t0
    }

    fn since(&self, other: &Self) -> Duration {
        self.t0.sub(other.t0)
    }
}

impl Add<Duration> for TestInstant {
    type Output = TestInstant;

    fn add(self, rhs: Duration) -> Self::Output {
        TestInstant {
            t0: self.t0.add(rhs),
        }
    }
}
impl Sub<Duration> for TestInstant {
    type Output = TestInstant;

    fn sub(self, rhs: Duration) -> Self::Output {
        TestInstant {
            t0: self.t0.sub(rhs),
        }
    }
}

impl Add for TestInstant {
    type Output = Duration;

    fn add(self, rhs: TestInstant) -> Duration {
        self.t0.add(rhs.t0)
    }
}

impl Sub for TestInstant {
    type Output = Duration;

    fn sub(self, rhs: TestInstant) -> Duration {
        self.t0.sub(rhs.t0)
    }
}
