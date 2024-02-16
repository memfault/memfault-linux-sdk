//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Duration;
use serde::Serialize;

use super::{MetricStringKey, MetricTimestamp};

use crate::util::serialization::float_to_duration;

#[derive(Copy, Clone, Debug, Serialize)]
pub enum MetricReading {
    /// Gauges are absolute values. We keep the latest value collected during a heartbeat.
    Gauge {
        value: f64,
        timestamp: MetricTimestamp,
        /// Time period considered for this reading. This is only used to give a "time-weight" to the first
        /// value in the series (when using time-weighted averages). For future values we will use the time
        /// difference we measure between the two points (because CollectD only
        /// provides the "configured" interval, not the actual interval between
        /// two readings).
        /// In doubt, it's safe to use Duration::from_secs(0) here. This means the first value will be ignored.
        #[serde(with = "float_to_duration")]
        interval: Duration,
    },
    /// A sum which always increases. The value is the difference between the current and the previous reading.
    /// We reset the Sum upon emitting a heartbeat.
    /// See Sum.Delta.Monotonic in https://opentelemetry.io/docs/specs/otel/metrics/data-model/#sums
    Counter {
        value: f64,
        timestamp: MetricTimestamp,
    },
}

#[derive(Debug, Serialize, Clone)]
pub struct KeyedMetricReading {
    pub name: MetricStringKey,
    pub value: MetricReading,
}

impl KeyedMetricReading {
    pub fn new(name: MetricStringKey, value: MetricReading) -> Self {
        Self { name, value }
    }
}
