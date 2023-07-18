//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Duration;
use serde::Serialize;

use super::{MetricStringKey, MetricTimestamp};

use crate::util::serialization::float_to_duration;

#[derive(Debug, Serialize)]
pub enum MetricReading {
    Gauge {
        value: f64,
        timestamp: MetricTimestamp,
    },
    Rate {
        value: f64,
        timestamp: MetricTimestamp,
        #[serde(with = "float_to_duration")]
        interval: Duration,
    },
}

#[derive(Serialize)]
pub struct KeyedMetricReading {
    pub name: MetricStringKey,
    pub value: MetricReading,
}
