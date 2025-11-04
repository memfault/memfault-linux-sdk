//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::time::{Duration, Instant};

use super::ScheduledTask;

pub struct Job {
    /// When is the next time we need to run this
    pub(super) next_run: Instant,
    /// How much time to wait after running it
    pub(super) period: Duration,
    /// Task
    pub(super) task: Box<dyn ScheduledTask>,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.next_run.eq(&other.next_run)
    }
}

impl Eq for Job {}
impl Ord for Job {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.next_run.cmp(&other.next_run).reverse()
    }
}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use crate::MailboxError;

    use super::*;
    use std::{
        collections::BinaryHeap,
        time::{Duration, Instant},
    };

    #[test]
    fn compare_jobs() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(10);

        let job1 = Job {
            next_run: t0,
            period: Duration::from_millis(10),
            task: Box::new(MockTask {}),
        };
        let job2 = Job {
            next_run: t1,
            period: Duration::from_millis(10),
            task: Box::new(MockTask {}),
        };

        assert!(job1 > job2);
        assert!(job1 != job2);
    }

    #[test]
    fn use_in_a_heap() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(10);

        let job1 = Job {
            next_run: t0,
            period: Duration::from_millis(10),
            task: Box::new(MockTask {}),
        };
        let job2 = Job {
            next_run: t1,
            period: Duration::from_millis(10),
            task: Box::new(MockTask {}),
        };

        let mut heap = BinaryHeap::new();
        heap.push(job1);
        heap.push(job2);
        assert_eq!(heap.peek().unwrap().next_run, t0);
    }

    #[test]
    fn can_have_multiple_jobs_at_same_time() {
        let t0 = Instant::now();

        let job1 = Job {
            next_run: t0,
            period: Duration::from_millis(10),
            task: Box::new(MockTask {}),
        };
        let job2 = Job {
            next_run: t0,
            period: Duration::from_millis(10),
            task: Box::new(MockTask {}),
        };

        // It was not clear to me from the docs whether two items that are equal
        // would be kept in the binary heap
        let mut heap = BinaryHeap::new();
        heap.push(job1);
        heap.push(job2);
        assert_eq!(heap.peek().unwrap().next_run, t0);

        assert_eq!(heap.len(), 2)
    }

    struct MockTask {}
    impl ScheduledTask for MockTask {
        fn execute(&self) -> Result<(), MailboxError> {
            Ok(())
        }
    }
}
