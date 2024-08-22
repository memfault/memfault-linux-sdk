//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{fmt::Display, time::Duration};

use super::DeliveryStats;

pub struct StatsAggregator {
    count: usize,
    max_queueing: Duration,
    max_processing: Duration,
    total_processing: Duration,
}

impl StatsAggregator {
    pub fn new() -> Self {
        StatsAggregator {
            count: 0,
            max_queueing: Duration::ZERO,
            max_processing: Duration::ZERO,
            total_processing: Duration::ZERO,
        }
    }
    pub fn add(&mut self, stats: &DeliveryStats) {
        self.count += 1;
        self.total_processing += stats.processing;
        self.max_queueing = self.max_queueing.max(stats.queued);
        self.max_processing = self.max_processing.max(stats.processing);
    }
}

impl Default for StatsAggregator {
    fn default() -> Self {
        StatsAggregator::new()
    }
}

impl Display for StatsAggregator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.count > 0 {
            f.write_fmt(format_args!(
                "Calls: {} Max Queueing: {} Processing (avg/max): {}/{} ",
                self.count,
                self.max_queueing.as_millis(),
                (self.total_processing / (self.count as u32)).as_millis(),
                self.max_processing.as_millis(),
            ))
        } else {
            f.write_fmt(format_args!("Calls: 0"))
        }
    }
}
