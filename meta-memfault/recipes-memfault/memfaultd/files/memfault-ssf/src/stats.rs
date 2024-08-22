//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::time::Duration;

mod aggregator;
pub use aggregator::*;

pub struct DeliveryStats {
    /// How long message was in queue before being delivered
    pub queued: Duration,
    /// Time to process message (excluding queueing time)
    pub processing: Duration,
}
