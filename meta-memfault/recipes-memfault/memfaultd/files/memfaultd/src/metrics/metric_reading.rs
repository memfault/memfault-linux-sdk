//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::str::FromStr;

use chrono::{Duration, Utc};
use eyre::{eyre, ErrReport};
use serde::{Deserialize, Serialize};

use super::{MetricStringKey, MetricTimestamp};

use crate::util::serialization::float_to_duration;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum MetricReading {
    /// TimeWeightedAverage readings will be aggregated based on
    /// the time the reading was captured over.
    TimeWeightedAverage {
        value: f64,
        timestamp: MetricTimestamp,
        /// Time period considered for this reading. This is only used to give a "time-weight" to the first
        /// value in the series (when using time-weighted averages). For future values we will use the time
        /// difference we measure between the two points         
        /// In doubt, it's safe to use Duration::from_secs(0) here. This means the first value will be ignored.
        #[serde(with = "float_to_duration")]
        interval: Duration,
    },
    /// A non-decreasing monotonic sum. Within a metric report, Counter readings are summed together.
    Counter {
        value: f64,
        timestamp: MetricTimestamp,
    },
    /// Gauges are absolute values. We keep the latest value collected during a metric report.
    Gauge {
        value: f64,
        timestamp: MetricTimestamp,
    },
    /// Histogram readings are averaged together by dividing the sum of the values
    /// by the number of readings over the duration of a metric report.
    Histogram {
        value: f64,
        timestamp: MetricTimestamp,
    },
}

#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct KeyedMetricReading {
    pub name: MetricStringKey,
    pub value: MetricReading,
}

impl KeyedMetricReading {
    pub fn new(name: MetricStringKey, value: MetricReading) -> Self {
        Self { name, value }
    }
}

impl FromStr for KeyedMetricReading {
    type Err = ErrReport;

    /// Deserialize a string in the form <MetricStringKey>=<f64> to a Gauge metric reading
    ///
    /// Currently deserialization to a KeyedMetricReading with any other type of MetricReading
    /// as its value is not supported
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (key, value_str) = s.split_once('=').ok_or(eyre!(
            "Gauge metric reading should be specified as KEY=VALUE"
        ))?;

        // Let's ensure the key is valid first:
        let metric_key = MetricStringKey::from_str(key).map_err(|e| eyre!(e))?;
        let value =
            f64::from_str(value_str).map_err(|e| eyre!("Invalid value {}: {}", value_str, e))?;

        let reading = MetricReading::Gauge {
            value,
            timestamp: Utc::now(),
        };
        Ok(KeyedMetricReading::new(metric_key, reading))
    }
}
