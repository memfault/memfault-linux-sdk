//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::str::FromStr;

use chrono::{Duration, Utc};
use eyre::{eyre, ErrReport};
use nom::{
    branch::alt,
    character::complete::char,
    combinator::value,
    number::complete::double,
    sequence::separated_pair,
    Finish,
    {bytes::complete::tag, IResult},
};
use serde::{Deserialize, Serialize};

use super::{MetricStringKey, MetricTimestamp};

use crate::util::serialization::float_to_duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
/// A typed value and timestamp pair that represents
/// an individual reading value for a metric. This type does
/// not have a notion of which key it is associated it
/// and is purely the "value" in a metric reading.
/// For the full metric reading type that includes the key,
/// use KeyedMetricReading.
pub enum MetricReading {
    /// TimeWeightedAverage readings will be aggregated based on
    /// the time the reading was captured over.
    TimeWeightedAverage {
        value: f64,
        timestamp: MetricTimestamp,
        /// Time period considered for this reading. This is only used to give a "time-weight" to the first
        /// value in the series. For future values we will use the time
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
    /// ReportTags are string values associated with the MetricReport they are captured in.
    /// We keep the latest value collected during a metric report for a key and drop the older
    /// ones.
    ReportTag {
        value: String,
        timestamp: MetricTimestamp,
    },
}

impl MetricReading {
    /// Parse a metric reading in the format f64|<StatsDMetricType>.
    /// The timestamp will be set to Utc::now().
    /// This is the suffix of a full StatsD reading of the following format:
    /// <MetricStringKey>:<MetricReading>
    ///
    /// Examples of valid readings:
    /// 64|h
    /// 100.0|c
    /// -89.5|g
    fn parse(input: &str) -> IResult<&str, MetricReading> {
        let (remaining, (value, statsd_type)) =
            separated_pair(double, tag("|"), StatsDMetricType::parse)(input)?;
        let timestamp = Utc::now();
        match statsd_type {
            StatsDMetricType::Histogram => {
                Ok((remaining, MetricReading::Histogram { value, timestamp }))
            }
            StatsDMetricType::Counter => {
                Ok((remaining, MetricReading::Counter { value, timestamp }))
            }
            StatsDMetricType::Timer => Ok((remaining, MetricReading::Counter { value, timestamp })),
            StatsDMetricType::Gauge => Ok((remaining, MetricReading::Gauge { value, timestamp })),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyedMetricReading {
    pub name: MetricStringKey,
    pub value: MetricReading,
}

impl KeyedMetricReading {
    pub fn new(name: MetricStringKey, value: MetricReading) -> Self {
        Self { name, value }
    }

    pub fn new_gauge(name: MetricStringKey, value: f64) -> Self {
        Self {
            name,
            value: MetricReading::Gauge {
                value,
                timestamp: Utc::now(),
            },
        }
    }

    pub fn new_histogram(name: MetricStringKey, value: f64) -> Self {
        Self {
            name,
            value: MetricReading::Histogram {
                value,
                timestamp: Utc::now(),
            },
        }
    }

    pub fn new_counter(name: MetricStringKey, value: f64) -> Self {
        Self {
            name,
            value: MetricReading::Counter {
                value,
                timestamp: Utc::now(),
            },
        }
    }

    /// Construct a KeyedMetricReading from a string in the StatsD format
    /// <MetricStringKey:<f64>|<StatsDMetricType>
    ///
    /// Examples of valid keyed metric readings:
    ///  testCounter:1|c
    ///  test_counter:1.0|c
    ///  test_histo:100|h
    ///  test_gauge:1.7|g
    ///  cpu3_idle:100.9898|g
    pub fn from_statsd_str(s: &str) -> Result<Self, ErrReport> {
        match Self::parse_statsd(s).finish() {
            Ok((_, reading)) => Ok(reading),
            Err(e) => Err(eyre!(
                "Failed to parse string \"{}\" as a StatsD metric reading: {}",
                s,
                e
            )),
        }
    }

    pub fn increment_counter(name: MetricStringKey) -> Self {
        Self {
            name,
            value: MetricReading::Counter {
                value: 1.0,
                timestamp: Utc::now(),
            },
        }
    }

    pub fn add_to_counter(name: MetricStringKey, value: f64) -> Self {
        Self {
            name,
            value: MetricReading::Counter {
                value,
                timestamp: Utc::now(),
            },
        }
    }

    /// Helper that handles `nom` details for parsing a StatsD string as
    /// a KeyedMetricReading
    fn parse_statsd(input: &str) -> IResult<&str, KeyedMetricReading> {
        let (remaining, (name, value)) =
            separated_pair(MetricStringKey::parse, tag(":"), MetricReading::parse)(input)?;
        Ok((remaining, KeyedMetricReading { name, value }))
    }

    /// Deserialize a string in the form <MetricStringKey>=<f64> to a Gauge metric reading
    ///
    /// Currently deserialization to a KeyedMetricReading with any other type of MetricReading
    /// as its value is not supported
    fn from_arg_str(s: &str) -> Result<Self, ErrReport> {
        let (key, value_str) = s.split_once('=').ok_or(eyre!(
            "Attached metric reading should be specified as KEY=VALUE"
        ))?;

        // Let's ensure the key is valid first:
        let metric_key = MetricStringKey::from_str(key).map_err(|e| eyre!(e))?;
        if let Ok(value) = f64::from_str(value_str) {
            let reading = MetricReading::Gauge {
                value,
                timestamp: Utc::now(),
            };
            Ok(KeyedMetricReading::new(metric_key, reading))
        } else {
            let reading = MetricReading::ReportTag {
                value: value_str.to_string(),
                timestamp: Utc::now(),
            };
            Ok(KeyedMetricReading::new(metric_key, reading))
        }
    }
}

impl FromStr for KeyedMetricReading {
    type Err = ErrReport;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match Self::from_statsd_str(s) {
            Ok(reading) => Ok(reading),
            Err(_) => match Self::from_arg_str(s) {
                Ok(reading) => Ok(reading),
                Err(e) => Err(eyre!("Couldn't parse \"{}\" as a Gauge metric: {}", s, e)),
            },
        }
    }
}

#[derive(Debug, Clone)]
enum StatsDMetricType {
    Counter,
    Histogram,
    Gauge,
    Timer,
}

impl StatsDMetricType {
    /// Parse a StatsDMetricType, which must be one of 'c', 'g', or 'h'
    ///
    /// 'c' indicates a Counter reading
    /// 'g' indicates a Gauge reading
    /// 'h' indicates a Histogram reading
    fn parse(input: &str) -> IResult<&str, StatsDMetricType> {
        alt((
            value(StatsDMetricType::Counter, char('c')),
            value(StatsDMetricType::Histogram, char('h')),
            value(StatsDMetricType::Gauge, char('g')),
            value(StatsDMetricType::Timer, tag("ms")),
        ))(input)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test_utils::setup_logger;
    use rstest::rstest;

    #[rstest]
    #[case("testCounter=1")]
    #[case("hello=world")]
    #[case("float=100.0")]
    fn parse_valid_arg_reading(#[case] reading_str: &str, _setup_logger: ()) {
        assert!(KeyedMetricReading::from_str(reading_str).is_ok())
    }

    #[rstest]
    #[case("testCounter:1|c")]
    #[case("test_counter:1.0|c")]
    #[case("test_histo:100|h")]
    #[case("test_gauge:1.7|g")]
    #[case("cpu3_idle:100.9898|g")]
    #[case("some_negative_gauge:-87.55|g")]
    #[case("test_timer:3600000|ms")]
    fn parse_valid_statsd_reading(#[case] reading_str: &str, _setup_logger: ()) {
        assert!(KeyedMetricReading::from_str(reading_str).is_ok())
    }

    #[rstest]
    #[case("test Counter:1|c")]
    #[case("{test_counter:1.0|c}")]
    #[case("\"test_counter\":1.0|c}")]
    #[case("test_gauge:\"string-value\"|g")]
    #[case("test_gauge:string-value|g")]
    fn fail_on_invalid_statsd_reading(#[case] reading_str: &str, _setup_logger: ()) {
        assert!(KeyedMetricReading::from_str(reading_str).is_err())
    }
}
