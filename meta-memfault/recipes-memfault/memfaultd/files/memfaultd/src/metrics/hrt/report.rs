//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Contains the in memory representation of an HRT report.
//!
//! The HRT report represents a collection of readings over a given time period. This report
//! differs from the standard metric report in that it keeps all readings over time and does
//! not represent an aggregate.

use std::{
    collections::{hash_map::Entry, HashMap},
    num::NonZeroU32,
};

use chrono::{DateTime, Utc};
use log::{debug, warn};

use super::schema::{DataType, Datum, HrtMetricType};
use crate::{
    metrics::{KeyedMetricReading, MetricReading, MetricStringKey, MetricTimestamp},
    util::rate_limiter::RateLimiter,
};

pub const HRT_DEFAULT_MAX_SAMPLES_PER_MIN: u32 = 750;

/// Represents the in memory format of high resolution telemetry data.
///
/// This is a helper struct that allows for easier access to the data in the report. It
/// differs from the on disk format in that it optimizes for ease of access over a more
/// compact representation.
pub struct HrtReport {
    pub start_time: MetricTimestamp,
    pub readings: HashMap<MetricStringKey, HrtReadingData>,
    pub rate_limiter: RateLimiter<DateTime<Utc>>,
}

impl HrtReport {
    pub fn new(max_readings_per_min: NonZeroU32) -> Self {
        let start_time = Utc::now();

        // Cap the configured rate limit at
        // 2x the default
        let rate_limit_upper_bound = NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN * 2)
            .expect("Default HRT limit should be nonzero");
        let rate_limit = if max_readings_per_min > rate_limit_upper_bound {
            debug!(
                "Configured HRT rate limit is {} - capping at {} samples per minute",
                max_readings_per_min, rate_limit_upper_bound
            );
            rate_limit_upper_bound
        } else {
            max_readings_per_min
        };

        Self {
            start_time,
            readings: HashMap::new(),
            rate_limiter: RateLimiter::new(rate_limit),
        }
    }

    pub fn add_metric(&mut self, reading: &KeyedMetricReading) {
        if let Err(e) =
            self.rate_limiter
                .run_within_limits(reading.value.timestamp(), |rate_limited_calls| {
                    if let Some(limited) = rate_limited_calls {
                        debug!("{} HRT readings were rate limited.", limited.count);
                    };
                    match self.readings.entry(reading.name.clone()) {
                        Entry::Occupied(mut o) => {
                            let hrt_data = o.get_mut();
                            hrt_data.push_reading(reading);
                            Ok(())
                        }
                        Entry::Vacant(v) => {
                            v.insert(HrtReadingData::from(reading));
                            Ok(())
                        }
                    }
                })
        {
            warn!("Failed to add HRT metric reading: {}", e);
        }
    }
}

impl Default for HrtReport {
    fn default() -> Self {
        let start_time = Utc::now();

        Self {
            start_time,
            readings: HashMap::new(),
            rate_limiter: RateLimiter::new(
                // Default
                NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                    .expect("Zero value passed to non-zero constructor"),
            ),
        }
    }
}

#[derive(Debug)]
/// Represents both reading data and metadata for a single metric key.
pub struct HrtReadingData {
    pub readings: Vec<Datum>,
    pub metadata: HrtReadingMetadata,
}

impl From<&KeyedMetricReading> for HrtReadingData {
    fn from(reading: &KeyedMetricReading) -> Self {
        Self {
            readings: vec![Datum::from(reading)],
            metadata: HrtReadingMetadata::from(reading),
        }
    }
}

impl HrtReadingData {
    fn push_reading(&mut self, reading: &KeyedMetricReading) {
        self.readings.push(Datum::from(reading))
    }
}

#[derive(Debug)]
/// Metadata for a single metric key.
pub struct HrtReadingMetadata {
    pub metric_type: HrtMetricType,
    pub data_type: DataType,
    pub internal: bool,
}

impl From<&KeyedMetricReading> for HrtReadingMetadata {
    fn from(reading: &KeyedMetricReading) -> Self {
        let (metric_type, data_type) = match reading.value {
            MetricReading::TimeWeightedAverage { .. } => (HrtMetricType::Gauge, DataType::Double),
            MetricReading::Histogram { .. } => (HrtMetricType::Gauge, DataType::Double),
            MetricReading::Counter { .. } => (HrtMetricType::Counter, DataType::Double),
            MetricReading::Gauge { .. } => (HrtMetricType::Gauge, DataType::Double),
            MetricReading::Rssi { .. } => (HrtMetricType::Gauge, DataType::Double),
            MetricReading::ReportTag { .. } => (HrtMetricType::Property, DataType::String),
            MetricReading::Bool { .. } => (HrtMetricType::Property, DataType::Boolean),
        };

        Self {
            metric_type,
            data_type,
            internal: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::metrics::hrt::schema::HighResTelemetryV1;

    use super::*;
    use insta::{assert_json_snapshot, with_settings};
    use rstest::rstest;

    #[rstest]
    fn test_build_hrt_report() {
        let mut hrt_report = HrtReport::new(NonZeroU32::new(2000).unwrap());

        for i in 0..100 {
            hrt_report.add_metric(&KeyedMetricReading::new_counter(
                MetricStringKey::from("test_counter"),
                i as f64,
            ))
        }

        for i in 1000..1100 {
            hrt_report.add_metric(&KeyedMetricReading::new_counter(
                MetricStringKey::from("test_counter_2"),
                i as f64,
            ))
        }

        for i in 500..1100 {
            hrt_report.add_metric(&KeyedMetricReading::new_histogram(
                MetricStringKey::from("test_histo"),
                i as f64,
            ))
        }

        for i in 200..330 {
            hrt_report.add_metric(&KeyedMetricReading::new_gauge(
                MetricStringKey::from("test_gauge"),
                i as f64,
            ))
        }

        let mut hrt_report_serialized = HighResTelemetryV1::try_from(hrt_report).unwrap();
        hrt_report_serialized.sort_rollups();
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(hrt_report_serialized,
                                  {".producer.version" => "[version]", ".start_time" => "[start_time]", ".rollups[].data[].t" => "[timestamp]", ".duration_ms" => "[duration]"});
        });
    }

    #[rstest]
    fn test_hrt_report_rate_limiting() {
        let mut hrt_report = HrtReport::default();

        for i in 0..1000 {
            hrt_report.add_metric(&KeyedMetricReading::new_counter(
                MetricStringKey::from("test_counter"),
                i as f64,
            ))
        }

        assert_eq!(
            hrt_report
                .readings
                .get(&MetricStringKey::from("test_counter"))
                .unwrap()
                .readings
                .len(),
            750
        );
    }
}
