//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Utc;
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    str::FromStr,
    time::{Duration, Instant},
};

use crate::{
    mar::{MarEntryBuilder, Metadata},
    metrics::{
        core_metrics::{
            METRIC_BATTERY_DISCHARGE_DURATION_MS, METRIC_BATTERY_SOC_PCT_DROP,
            METRIC_CONNECTED_TIME, METRIC_EXPECTED_CONNECTED_TIME, METRIC_MF_SYNC_FAILURE,
            METRIC_MF_SYNC_SUCCESS, METRIC_OPERATIONAL_CRASHES, METRIC_SYNC_FAILURE,
            METRIC_SYNC_SUCCESS,
        },
        metric_reading::KeyedMetricReading,
        timeseries::{Counter, Gauge, Histogram, TimeSeries, TimeWeightedAverage},
        MetricReading, MetricStringKey, MetricValue, SessionName,
    },
};

use super::timeseries::ReportTag;

pub enum CapturedMetrics {
    All,
    Metrics(HashSet<MetricStringKey>),
}

pub const HEARTBEAT_REPORT_TYPE: &str = "heartbeat";
pub const DAILY_HEARTBEAT_REPORT_TYPE: &str = "daily-heartbeat";

const SESSION_CORE_METRICS: &[&str; 9] = &[
    METRIC_MF_SYNC_FAILURE,
    METRIC_MF_SYNC_SUCCESS,
    METRIC_BATTERY_DISCHARGE_DURATION_MS,
    METRIC_BATTERY_SOC_PCT_DROP,
    METRIC_CONNECTED_TIME,
    METRIC_EXPECTED_CONNECTED_TIME,
    METRIC_SYNC_FAILURE,
    METRIC_SYNC_SUCCESS,
    METRIC_OPERATIONAL_CRASHES,
];

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum MetricReportType {
    #[serde(rename = "heartbeat")]
    Heartbeat,
    #[serde(rename = "session")]
    Session(SessionName),
    #[serde(rename = "daily-heartbeat")]
    DailyHeartbeat,
}

impl MetricReportType {
    pub fn as_str(&self) -> &str {
        match self {
            MetricReportType::Heartbeat => HEARTBEAT_REPORT_TYPE,
            MetricReportType::Session(session_name) => session_name.as_str(),
            MetricReportType::DailyHeartbeat => DAILY_HEARTBEAT_REPORT_TYPE,
        }
    }
}

pub struct MetricReport {
    /// In-memory metric store for this report
    metrics: HashMap<MetricStringKey, Box<dyn TimeSeries + Send>>,
    /// Point in time when capture of metrics currently in
    /// report's metric store began
    start: Instant,
    /// Configuration of which metrics this report should capture
    captured_metrics: CapturedMetrics,
    /// Indicates whether this is a heartbeat metric report or
    /// session metric report (with session name)
    report_type: MetricReportType,
}

struct MetricReportSnapshot {
    duration: Duration,
    metrics: HashMap<MetricStringKey, MetricValue>,
}

impl MetricReport {
    pub fn new(
        report_type: MetricReportType,
        configured_captured_metrics: CapturedMetrics,
    ) -> Self {
        // Always include session core metrics regardless of configuration
        let captured_metrics = match configured_captured_metrics {
            CapturedMetrics::All => CapturedMetrics::All,
            CapturedMetrics::Metrics(metrics) => {
                let mut merged_set = SESSION_CORE_METRICS
                    .iter()
                    .map(|core_metric_key| {
                        // This expect should never be hit as SESSION_CORE_METRICS is
                        // an array of static strings
                        MetricStringKey::from_str(core_metric_key)
                            .expect("Invalid Metric Key in SESSION_CORE_METRICS")
                    })
                    .collect::<HashSet<_>>();
                merged_set.extend(metrics);
                CapturedMetrics::Metrics(merged_set)
            }
        };

        Self {
            metrics: HashMap::new(),
            start: Instant::now(),
            captured_metrics,
            report_type,
        }
    }

    /// Creates a heartbeat report that captures all metrics
    pub fn new_heartbeat() -> Self {
        MetricReport::new(MetricReportType::Heartbeat, CapturedMetrics::All)
    }

    /// Creates a daily heartbeat report that captures all metrics
    pub fn new_daily_heartbeat() -> Self {
        MetricReport::new(MetricReportType::DailyHeartbeat, CapturedMetrics::All)
    }

    fn is_captured(&self, metric_key: &MetricStringKey) -> bool {
        match &self.captured_metrics {
            CapturedMetrics::Metrics(metric_keys) => metric_keys.contains(metric_key),
            CapturedMetrics::All => true,
        }
    }

    /// Adds a metric reading to the report's internal
    /// metric store if the report captures that metric,
    /// otherwise no-op
    pub fn add_metric(&mut self, m: KeyedMetricReading) -> Result<()> {
        if self.is_captured(&m.name) {
            match self.metrics.entry(m.name) {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    let state = o.get_mut();
                    if let Err(e) = (*state).aggregate(&m.value) {
                        *state = Self::select_aggregate_for(&m.value)?;
                        log::warn!(
                            "New value for metric {} is incompatible ({}). Resetting timeseries.",
                            o.key(),
                            e
                        );
                    }
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    let timeseries = Self::select_aggregate_for(&m.value)?;
                    v.insert(timeseries);
                }
            };
        }
        Ok(())
    }

    /// Increment a counter metric by 1
    pub fn increment_counter(&mut self, name: &str) -> Result<()> {
        self.add_to_counter(name, 1.0)
    }

    pub fn add_to_counter(&mut self, name: &str, value: f64) -> Result<()> {
        match name.parse::<MetricStringKey>() {
            Ok(metric_name) => self.add_metric(KeyedMetricReading::new(
                metric_name,
                MetricReading::Counter {
                    value,
                    timestamp: Utc::now(),
                },
            )),
            Err(e) => Err(eyre!("Invalid metric name: {} - {}", name, e)),
        }
    }

    /// Return all the metrics in memory for this report and resets its store.
    pub fn take_metrics(&mut self) -> HashMap<MetricStringKey, MetricValue> {
        self.take_metric_report_snapshot().metrics
    }

    fn take_metric_report_snapshot(&mut self) -> MetricReportSnapshot {
        let duration = std::mem::replace(&mut self.start, Instant::now()).elapsed();
        let metrics = std::mem::take(&mut self.metrics)
            .into_iter()
            .map(|(name, state)| (name, state.value()))
            .collect();

        MetricReportSnapshot { duration, metrics }
    }

    /// Create one metric report MAR entry with all the metrics in the store.
    ///
    /// All data will be timestamped with current time measured by CollectionTime::now(), effectively
    /// disregarding the collectd timestamps.
    pub fn prepare_metric_report(
        &mut self,
        mar_staging_area: &Path,
    ) -> Result<Option<MarEntryBuilder<Metadata>>> {
        let snapshot = self.take_metric_report_snapshot();

        if snapshot.metrics.is_empty() {
            return Ok(None);
        }

        Ok(Some(MarEntryBuilder::new(mar_staging_area)?.set_metadata(
            Metadata::new_metric_report(
                snapshot.metrics,
                snapshot.duration,
                self.report_type.clone(),
            ),
        )))
    }

    fn select_aggregate_for(event: &MetricReading) -> Result<Box<dyn TimeSeries + Send>> {
        match event {
            MetricReading::Histogram { .. } => Ok(Box::new(Histogram::new(event)?)),
            MetricReading::Counter { .. } => Ok(Box::new(Counter::new(event)?)),
            MetricReading::Gauge { .. } => Ok(Box::new(Gauge::new(event)?)),
            MetricReading::TimeWeightedAverage { .. } => {
                Ok(Box::new(TimeWeightedAverage::new(event)?))
            }
            MetricReading::ReportTag { .. } => Ok(Box::new(ReportTag::new(event)?)),
        }
    }

    pub fn report_type(&self) -> &MetricReportType {
        &self.report_type
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use std::collections::BTreeMap;

    use super::*;
    use crate::test_utils::in_histograms;
    use std::str::FromStr;

    use insta::assert_json_snapshot;
    use rstest::rstest;

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz",  3.0)]), "heartbeat_report_1")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0), ("foo",  3.0)]), "heartbeat_report_2")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 1.0)]), "heartbeat_report_3")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0)]), "heartbeat_report_4")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0), ("foo",  2.0)]), "heartbeat_report_5")]
    fn test_aggregate_metrics(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let mut metric_report = MetricReport::new_heartbeat();

        for m in metrics {
            metric_report.add_metric(m).unwrap();
        }
        let sorted_metrics: BTreeMap<_, _> = metric_report.take_metrics().into_iter().collect();
        assert_json_snapshot!(test_name, sorted_metrics);
    }

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz",  3.0)]), "session_report_1")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0), ("foo",  3.0)]), "session_report_2")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 1.0)]), "session_report_3")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0)]), "session_report_4")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0), ("baz",  2.0), ("bar",  3.0)]), "session_report_5")]
    fn test_aggregate_metrics_session(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let mut metric_report = MetricReport::new(
            MetricReportType::Session(SessionName::from_str("foo_only").unwrap()),
            CapturedMetrics::Metrics(
                [
                    MetricStringKey::from_str("foo").unwrap(),
                    MetricStringKey::from_str("baz").unwrap(),
                ]
                .into_iter()
                .collect(),
            ),
        );

        for m in metrics {
            metric_report.add_metric(m).unwrap();
        }
        let sorted_metrics: BTreeMap<_, _> = metric_report.take_metrics().into_iter().collect();
        assert_json_snapshot!(test_name, sorted_metrics);
    }

    /// Core metrics should always be captured by sessions even if they are not
    /// configured to do so
    #[rstest]
    fn test_aggregate_core_metrics_session() {
        let mut metric_report = MetricReport::new(
            MetricReportType::Session(SessionName::from_str("foo_only").unwrap()),
            CapturedMetrics::Metrics(
                [
                    MetricStringKey::from_str("foo").unwrap(),
                    MetricStringKey::from_str("baz").unwrap(),
                ]
                .into_iter()
                .collect(),
            ),
        );

        let metrics = in_histograms(
            SESSION_CORE_METRICS
                .map(|metric_name: &'static str| (metric_name, 100.0))
                .to_vec(),
        );

        for m in metrics {
            metric_report.add_metric(m).unwrap();
        }
        let sorted_metrics: BTreeMap<_, _> = metric_report.take_metrics().into_iter().collect();
        assert_json_snapshot!(sorted_metrics);
    }

    #[rstest]
    fn test_empty_after_write() {
        let mut metric_report = MetricReport::new_heartbeat();
        for m in in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz", 3.0)]) {
            metric_report.add_metric(m).unwrap();
        }

        let tempdir = TempDir::new().unwrap();
        let _ = metric_report.prepare_metric_report(tempdir.path());
        assert_eq!(metric_report.take_metrics().len(), 0);
    }
}
