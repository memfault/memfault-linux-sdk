//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};
use eyre::{eyre, Result};
use std::cmp;

use super::{construct_histogram_value, MetricReading, MetricValue};

const FINITENESS_ERROR: &str = "Metric values must be finite.";

/// A trait for the storage of multiple metric events aggregated together.
/// This (roughly) maps to a time series in OpenTelemetry data model:
/// https://opentelemetry.io/docs/specs/otel/metrics/data-model/#timeseries-model
pub trait TimeSeries {
    fn aggregate(&mut self, newer: &MetricReading) -> Result<()>;
    fn value(&self) -> MetricValue;
}

pub struct Histogram {
    sum: f64,
    count: u64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    min: f64,
    max: f64,
}

impl Histogram {
    pub fn new(reading: &MetricReading) -> Result<Self> {
        match *reading {
            MetricReading::Histogram { value, timestamp } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }

                Ok(Self {
                    sum: value,
                    count: 1,
                    start: timestamp,
                    end: timestamp,
                    min: value,
                    max: value,
                })
            }
            _ => Err(eyre!("Cannot create a histogram from a non-gauge metric")),
        }
    }
}

impl TimeSeries for Histogram {
    fn aggregate(&mut self, newer: &MetricReading) -> Result<()> {
        match newer {
            MetricReading::Histogram { value, timestamp } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                self.sum += value;
                self.count += 1;
                self.start = cmp::min(self.start, *timestamp);
                self.end = cmp::max(self.end, *timestamp);
                self.min = f64::min(self.min, *value);
                self.max = f64::max(self.max, *value);
                Ok(())
            }
            _ => Err(eyre!(
                "Cannot aggregate a histogram with a non-gauge metric"
            )),
        }
    }

    fn value(&self) -> MetricValue {
        if self.count > 0 {
            MetricValue::Histogram(construct_histogram_value(
                self.min,
                self.sum / self.count as f64,
                self.max,
            ))
        } else {
            MetricValue::Histogram(construct_histogram_value(f64::NAN, f64::NAN, f64::NAN))
        }
    }
}

/// An aggregation that calculates the sum of all values received. This assumes that all readings will be positive numbers.
/// Monotonic counter in OpenTelemetry data model.
pub struct Counter {
    sum: f64,
    end: DateTime<Utc>,
}

impl Counter {
    pub fn new(reading: &MetricReading) -> Result<Self> {
        match *reading {
            MetricReading::Counter { value, timestamp } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                Ok(Self {
                    sum: value,
                    end: timestamp,
                })
            }
            _ => Err(eyre!("Cannot create a sum from a non-counter metric")),
        }
    }
}

impl TimeSeries for Counter {
    fn aggregate(&mut self, newer: &MetricReading) -> Result<()> {
        match newer {
            MetricReading::Counter { value, timestamp } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                self.sum += value;
                self.end = *timestamp;
                Ok(())
            }
            _ => Err(eyre!("Cannot aggregate a sum with a non-counter metric")),
        }
    }

    fn value(&self) -> MetricValue {
        MetricValue::Number(self.sum)
    }
}

/// A time-weighted sum of all values received. This is useful to maintain an accurate average measurement when the interval between readings is not constant.
pub struct TimeWeightedAverage {
    weighted_sum: f64,
    duration: u64,
    end: DateTime<Utc>,
}

impl TimeWeightedAverage {
    pub fn new(reading: &MetricReading) -> Result<Self> {
        match *reading {
            MetricReading::TimeWeightedAverage {
                value,
                timestamp,
                interval,
            } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                Ok(Self {
                    weighted_sum: value * interval.num_milliseconds() as f64,
                    duration: interval.num_milliseconds() as u64,
                    end: timestamp,
                })
            }
            _ => Err(eyre!(
                "Mismatch between Time Weighted Average aggregation and {:?} reading",
                reading
            )),
        }
    }
}

impl TimeSeries for TimeWeightedAverage {
    fn aggregate(&mut self, newer: &MetricReading) -> Result<()> {
        match newer {
            MetricReading::TimeWeightedAverage {
                value, timestamp, ..
            } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                if timestamp < &self.end {
                    return Err(eyre!(
                        "Cannot aggregate a time-weighted average with an older timestamp"
                    ));
                }
                let duration = (*timestamp - self.end).num_milliseconds() as u64;
                self.weighted_sum += value * duration as f64;
                self.duration += duration;
                self.end = *timestamp;
                Ok(())
            }
            _ => Err(eyre!(
                "Cannot aggregate a time-weighted average with a non-gauge metric"
            )),
        }
    }

    fn value(&self) -> MetricValue {
        if self.duration > 0 {
            MetricValue::Number(self.weighted_sum / self.duration as f64)
        } else {
            MetricValue::Number(f64::NAN)
        }
    }
}

pub struct Gauge {
    value: f64,
    end: DateTime<Utc>,
}

impl Gauge {
    pub fn new(reading: &MetricReading) -> Result<Self> {
        match *reading {
            MetricReading::Gauge { value, timestamp } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                Ok(Self {
                    value,
                    end: timestamp,
                })
            }
            _ => Err(eyre!(
                "Cannot create a gauge aggregation from a non-gauge metric"
            )),
        }
    }
}

impl TimeSeries for Gauge {
    fn aggregate(&mut self, newer: &MetricReading) -> Result<()> {
        match newer {
            MetricReading::Gauge { value, timestamp } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                if *timestamp > self.end {
                    self.value = *value;
                    self.end = *timestamp;
                }
                Ok(())
            }
            _ => Err(eyre!(
                "Cannot aggregate a histogram with a non-gauge metric"
            )),
        }
    }

    fn value(&self) -> MetricValue {
        MetricValue::Number(self.value)
    }
}

/// An aggregation stores the most recently received String
/// associated with a key as a tag on the report
pub struct ReportTag {
    value: String,
    end: DateTime<Utc>,
}

impl ReportTag {
    pub fn new(reading: &MetricReading) -> Result<Self> {
        match reading {
            MetricReading::ReportTag { value, timestamp } => Ok(Self {
                value: value.clone(),
                end: *timestamp,
            }),
            _ => Err(eyre!(
                "Cannot create a report tag from a non-report tag reading"
            )),
        }
    }
}

impl TimeSeries for ReportTag {
    fn aggregate(&mut self, newer: &MetricReading) -> Result<()> {
        match newer {
            MetricReading::ReportTag { value, timestamp } => {
                if *timestamp > self.end {
                    self.value = value.clone();
                    self.end = *timestamp;
                }
                Ok(())
            }
            _ => Err(eyre!(
                "Cannot aggregate a report tag with a non-report-tag reading"
            )),
        }
    }

    fn value(&self) -> MetricValue {
        MetricValue::String(self.value.clone())
    }
}
#[cfg(test)]
mod tests {
    use chrono::Duration;
    use rstest::rstest;

    use crate::metrics::{HistogramValue, MetricReading, MetricTimestamp, MetricValue};
    use std::{f64::INFINITY, f64::NAN, f64::NEG_INFINITY, str::FromStr};

    use super::TimeSeries;
    use super::{Counter, Gauge, Histogram, TimeWeightedAverage};

    #[rstest]
    #[case(1.0, 1000, 2.0, 1.5, 2.0, 1.0, 1000)]
    #[case(10.0, 10_000, 10.0, 10.0, 10.0, 10.0, 10_000)]
    #[case(1.0, 9_000, 0.0, 0.5, 1.0, 0.0, 9_000)]
    #[case(1.0, 0, 2.0, 1.5, 2.0, 1.0, 0)]
    fn test_histogram_aggregation(
        #[case] a: f64,
        #[case] duration_between_ms: i64,
        #[case] b: f64,
        #[case] expected_mean: f64,
        #[case] expected_max: f64,
        #[case] expected_min: f64,
        #[case] expected_ms: i64,
    ) {
        let t0 = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Histogram {
            value: a,
            timestamp: t0,
        };
        let b = MetricReading::Histogram {
            value: b,
            timestamp: t0 + Duration::milliseconds(duration_between_ms),
        };

        let mut h = Histogram::new(&a).unwrap();
        h.aggregate(&b).unwrap();

        assert_eq!(h.start, t0);
        assert_eq!(h.end, t0 + Duration::milliseconds(duration_between_ms));
        assert_eq!((h.end - h.start).num_milliseconds(), expected_ms);
        assert_eq!(
            h.value(),
            MetricValue::Histogram(HistogramValue {
                min: expected_min,
                mean: expected_mean,
                max: expected_max,
            })
        );
    }

    #[rstest]
    #[case(1.0, 1000, 2.0, 1000, 1.5, 2000)]
    #[case(10.0, 10000, 10.0, 1000, 10.0, 11000)]
    #[case(1.0, 9_000, 0.0, 1_000, 0.9, 10_000)]
    #[case(1.0, 0, 2.0, 1, 2.0, 1)]
    #[case(1.0, 1000, 2.0, 0, 1.0, 1000)]
    fn test_time_weighted_aggregation(
        #[case] a: f64,
        #[case] a_ms: i64,
        #[case] b: f64,
        #[case] b_ms: i64,
        #[case] expected: f64,
        #[case] expected_ms: u64,
    ) {
        let t0 = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();

        let a = MetricReading::TimeWeightedAverage {
            value: a,
            timestamp: t0 + Duration::milliseconds(a_ms),
            interval: Duration::milliseconds(a_ms),
        };
        let b = MetricReading::TimeWeightedAverage {
            value: b,
            timestamp: t0 + Duration::milliseconds(a_ms + b_ms),
            interval: Duration::milliseconds(b_ms),
        };

        let mut h = TimeWeightedAverage::new(&a).unwrap();
        h.aggregate(&b).unwrap();

        assert_eq!(h.end, t0 + Duration::milliseconds(a_ms + b_ms));
        assert_eq!(h.duration, expected_ms);
        assert_eq!(h.value(), MetricValue::Number(expected));
    }

    #[rstest]
    fn test_incompatible_metric_type_on_histogram() {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Histogram {
            value: 1.0,
            timestamp,
        };
        let b = MetricReading::Counter {
            value: 2.0,
            timestamp,
        };

        assert!(Histogram::new(&a).unwrap().aggregate(&b).is_err());
    }

    #[rstest]
    #[case(INFINITY)]
    #[case(NEG_INFINITY)]
    #[case(NAN)]
    fn test_edge_values_new(#[case] edge_value: f64) {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let a = MetricReading::Histogram {
            value: edge_value,
            timestamp,
        };
        assert!(Histogram::new(&a).is_err());
    }

    #[rstest]
    #[case(INFINITY)]
    #[case(NEG_INFINITY)]
    #[case(NAN)]
    fn test_edge_values_aggregate(#[case] edge_value: f64) {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let a = MetricReading::Histogram {
            value: 0.0,
            timestamp,
        };
        let b = MetricReading::Histogram {
            value: edge_value,
            timestamp,
        };
        assert!(Histogram::new(&a).unwrap().aggregate(&b).is_err());
    }

    #[rstest]
    #[case(1.0, 2.0, 3.0)]
    fn test_counter_aggregation(#[case] a: f64, #[case] b: f64, #[case] expected: f64) {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let timestamp2 = MetricTimestamp::from_str("2021-01-01T00:00:43Z").unwrap();

        let a = MetricReading::Counter {
            value: a,
            timestamp,
        };
        let b = MetricReading::Counter {
            value: b,
            timestamp: timestamp2,
        };

        let mut sum = Counter::new(&a).unwrap();
        sum.aggregate(&b).unwrap();
        assert_eq!(sum.end, timestamp2);
        assert_eq!(sum.sum, expected);
    }

    #[rstest]
    #[case(1.0, 2.0, 2.0)]
    #[case(10.0, 1.0, 1.0)]
    fn test_gauge_aggregation(#[case] a: f64, #[case] b: f64, #[case] expected: f64) {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let timestamp2 = MetricTimestamp::from_str("2021-01-01T00:00:43Z").unwrap();

        let a = MetricReading::Gauge {
            value: a,
            timestamp,
        };
        let b = MetricReading::Gauge {
            value: b,
            timestamp: timestamp2,
        };

        let mut gauge = Gauge::new(&a).unwrap();
        gauge.aggregate(&b).unwrap();
        assert_eq!(gauge.end, timestamp2);
        assert_eq!(gauge.value, expected);
    }
}
