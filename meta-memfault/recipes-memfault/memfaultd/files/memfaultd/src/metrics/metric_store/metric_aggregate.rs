//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Duration, Utc};
use eyre::{eyre, Result};
use serde::{Serialize, Serializer};

use super::{MetricReading, MetricValue};

const FINITENESS_ERROR: &str = "Metric values must be finite.";

#[derive(Debug)]
pub enum MetricAggregate {
    Latest {
        value: f64,
        last_update: DateTime<Utc>,
    },
    TimeWeightedAverage {
        value: f64,
        interval: Duration,
        last_update: DateTime<Utc>,
    },
}

impl MetricAggregate {
    pub fn new(reading: &MetricReading) -> Result<Self> {
        match reading {
            MetricReading::Gauge {
                value, timestamp, ..
            } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                Ok(Self::Latest {
                    value: *value,
                    last_update: *timestamp,
                })
            }
            MetricReading::Rate {
                value,
                interval: period,
                timestamp,
                ..
            } => {
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                Ok(Self::TimeWeightedAverage {
                    value: *value,
                    interval: *period,
                    last_update: *timestamp,
                })
            }
        }
    }

    /// Combine the current state of a metric with a new reading to calculate
    /// the new value.
    pub fn aggregate(&self, newer: &MetricReading) -> Result<Self> {
        match (self, newer) {
            (
                Self::Latest {
                    last_update: old_ts,
                    ..
                },
                MetricReading::Gauge {
                    value, timestamp, ..
                },
            ) => {
                if timestamp < old_ts {
                    return Err(eyre!(
                        "Cannot update metric with older timestamp: {:?} < {:?}",
                        timestamp,
                        old_ts
                    ));
                }
                if !value.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                Ok(Self::Latest {
                    value: *value,
                    last_update: *timestamp,
                })
            }
            (
                Self::TimeWeightedAverage {
                    value: a,
                    interval: ai,
                    last_update: at,
                },
                MetricReading::Rate {
                    value: b,
                    interval: _bi,
                    timestamp: bt,
                    ..
                },
            ) => {
                // We use the difference between the last update of the current state and the current time to update the current state.
                // This is more precise than relying on interval which could be inaccurate if a data collection was forced.
                if *bt <= *at {
                    return Err(eyre!(
                        "New data must be newer than existing data: {:?} < {:?}",
                        bt,
                        at
                    ));
                }
                if !b.is_finite() {
                    return Err(eyre!(FINITENESS_ERROR));
                }
                let t = *bt - *at;
                Ok(Self::TimeWeightedAverage {
                    value: (*a * ai.num_milliseconds() as f64 + b * t.num_milliseconds() as f64)
                        / (*ai + t).num_milliseconds() as f64,
                    interval: *ai + t,
                    last_update: *bt,
                })
            }
            _ => Err(eyre!(
                "Cannot coalesce metrics of different types: {:?} and {:?}",
                self,
                newer
            )),
        }
    }

    pub fn value(&self) -> MetricValue {
        match self {
            MetricAggregate::Latest { value: v, .. } => MetricValue::Number(*v),
            MetricAggregate::TimeWeightedAverage { value: v, .. } => MetricValue::Number(*v),
        }
    }
}

impl Serialize for MetricAggregate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MetricAggregate::Latest { value: v, .. } => serializer.serialize_f64(*v),
            MetricAggregate::TimeWeightedAverage { value: v, .. } => serializer.serialize_f64(*v),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use rstest::rstest;

    use crate::metrics::{MetricReading, MetricTimestamp};
    use std::{f64::INFINITY, f64::NAN, f64::NEG_INFINITY, str::FromStr};

    use super::MetricAggregate;

    #[rstest]
    #[case(1.0, 2.0, 2.0)]
    fn test_gauge_agregation(#[case] a: f64, #[case] b: f64, #[case] expected: f64) {
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

        let state = MetricAggregate::new(&a).unwrap().aggregate(&b).unwrap();
        match state {
            MetricAggregate::Latest { last_update, value } => {
                assert_eq!(last_update, timestamp2);
                assert_eq!(value, expected);
            }
            _ => panic!("Expected a rate"),
        }
    }

    #[rstest]
    #[case(1.0, 1000, 2.0, 1000, 1.5, 2000)]
    #[case(10.0, 10000, 10.0, 1000, 10.0, 11000)]
    #[case(1.0, 9_000, 0.0, 1_000, 0.9, 10_000)]
    fn test_rate_agregation(
        #[case] a: f64,
        #[case] a_ms: i64,
        #[case] b: f64,
        #[case] b_ms: i64,
        #[case] expected: f64,
        #[case] expected_ms: i64,
    ) {
        let t0 = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Rate {
            value: a,
            interval: Duration::milliseconds(a_ms),
            timestamp: t0 + Duration::milliseconds(a_ms),
        };
        let b = MetricReading::Rate {
            value: b,
            interval: Duration::milliseconds(b_ms),
            timestamp: t0 + Duration::milliseconds(a_ms + b_ms),
        };

        let state = MetricAggregate::new(&a).unwrap().aggregate(&b).unwrap();
        match state {
            MetricAggregate::TimeWeightedAverage {
                interval,
                last_update,
                value,
            } => {
                assert_eq!(interval.num_milliseconds(), expected_ms);
                assert_eq!(last_update, t0 + Duration::milliseconds(a_ms + b_ms));
                assert_eq!(value, expected);
            }
            _ => panic!("Expected a rate"),
        }
    }

    #[rstest]
    fn test_time_going_back_gauge() {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let timestamp2 = MetricTimestamp::from_str("2020-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Gauge {
            value: 1.0,
            timestamp,
        };
        let b = MetricReading::Gauge {
            value: 2.0,
            timestamp: timestamp2,
        };

        assert!(MetricAggregate::new(&a).unwrap().aggregate(&b).is_err());
    }

    #[rstest]
    fn test_time_going_back_rate() {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let timestamp2 = MetricTimestamp::from_str("2020-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Rate {
            value: 1.0,
            interval: Duration::milliseconds(1000),
            timestamp,
        };
        let b = MetricReading::Rate {
            value: 2.0,
            interval: Duration::milliseconds(1000),
            timestamp: timestamp2,
        };

        assert!(MetricAggregate::new(&a).unwrap().aggregate(&b).is_err());
    }

    #[rstest]
    fn test_time_difference_zero() {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Rate {
            value: 1.0,
            interval: Duration::milliseconds(1000),
            timestamp,
        };
        let b = MetricReading::Rate {
            value: 2.0,
            interval: Duration::milliseconds(1000),
            timestamp,
        };

        assert!(MetricAggregate::new(&a).unwrap().aggregate(&b).is_err());
    }

    #[rstest]
    fn test_incompatible_metric_type() {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();

        let a = MetricReading::Rate {
            value: 1.0,
            interval: Duration::milliseconds(1000),
            timestamp,
        };
        let b = MetricReading::Gauge {
            value: 2.0,
            timestamp,
        };

        assert!(MetricAggregate::new(&a).unwrap().aggregate(&b).is_err());
    }

    #[rstest]
    #[case(INFINITY)]
    #[case(NEG_INFINITY)]
    #[case(NAN)]
    fn test_edge_values_new(#[case] edge_value: f64) {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let interval = Duration::milliseconds(1000);
        let a = MetricReading::Rate {
            value: edge_value,
            timestamp,
            interval,
        };
        assert!(MetricAggregate::new(&a).is_err());
    }

    #[rstest]
    #[case(INFINITY)]
    #[case(NEG_INFINITY)]
    #[case(NAN)]
    fn test_edge_values_aggregate(#[case] edge_value: f64) {
        let timestamp = MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap();
        let interval = Duration::milliseconds(1000);
        let a = MetricReading::Rate {
            value: 0.0,
            timestamp,
            interval,
        };
        let b = MetricReading::Rate {
            value: edge_value,
            timestamp,
            interval,
        };
        assert!(MetricAggregate::new(&a).unwrap().aggregate(&b).is_err());
    }
}
