//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Utc;
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

use crate::{
    mar::{MarEntryBuilder, Metadata},
    metrics::{
        metric_reading::KeyedMetricReading,
        timeseries::{Counter, Gauge, Histogram, TimeSeries, TimeWeightedAverage},
        MetricReading, MetricStringKey, MetricValue, SessionName,
    },
    util::{system::get_system_clock, wildcard_pattern::WildcardPattern},
};

use super::{
    core_metrics::{
        METRIC_CPU_USAGE_PCT, METRIC_CPU_USAGE_PROCESS_PCT_PREFIX,
        METRIC_CPU_USAGE_PROCESS_PCT_SUFFIX, METRIC_MEMORY_PCT, METRIC_MEMORY_PROCESS_PCT_PREFIX,
        METRIC_MEMORY_PROCESS_PCT_SUFFIX,
    },
    system_metrics::{
        METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX, METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX,
        NETWORK_INTERFACE_METRIC_NAMESPACE, THERMAL_METRIC_NAMESPACE,
    },
    timeseries::{Bool, ReportTag, RssiAverage},
};

pub enum CapturedMetrics {
    All,
    Metrics(MetricsSet),
}

pub struct MetricsSet {
    pub metric_keys: HashSet<MetricStringKey>,
    pub wildcard_metric_keys: Vec<WildcardPattern>,
}

/// Returns a MetricsSet that represents
/// all the keys that should report a
/// min and max when aggregated as a
/// Histogram
fn histo_min_max_keys() -> MetricsSet {
    MetricsSet {
        metric_keys: HashSet::from_iter([
            MetricStringKey::from(METRIC_CPU_USAGE_PCT),
            MetricStringKey::from(METRIC_MEMORY_PCT),
        ]),
        wildcard_metric_keys: vec![
            // cpu_usage_*_pct
            WildcardPattern::new(
                METRIC_CPU_USAGE_PROCESS_PCT_PREFIX,
                METRIC_CPU_USAGE_PROCESS_PCT_SUFFIX,
            ),
            // memory_*_pct
            WildcardPattern::new(
                METRIC_MEMORY_PROCESS_PCT_PREFIX,
                METRIC_MEMORY_PROCESS_PCT_SUFFIX,
            ),
            // interface/*/bytes_per_second/rx
            WildcardPattern::new(
                NETWORK_INTERFACE_METRIC_NAMESPACE,
                METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX,
            ),
            // interface/*/bytes_per_second/tx
            WildcardPattern::new(
                NETWORK_INTERFACE_METRIC_NAMESPACE,
                METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX,
            ),
            // thermal/*
            WildcardPattern::new(THERMAL_METRIC_NAMESPACE, ""),
        ],
    }
}

impl MetricsSet {
    pub fn contains(&self, metric_string_key: &MetricStringKey) -> bool {
        self.metric_keys.contains(metric_string_key)
            || self
                .wildcard_metric_keys
                .iter()
                .any(|pattern| pattern.matches(metric_string_key.as_str()))
    }
}

pub const HEARTBEAT_REPORT_TYPE: &str = "heartbeat";
pub const DAILY_HEARTBEAT_REPORT_TYPE: &str = "daily-heartbeat";

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
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
    /// Point in time when capture of metrics currently in
    /// report's metric store according to CLOCK_BOOTTIME
    boottime_start: Option<Duration>,
    /// Configuration of which metrics this report should capture
    captured_metrics: CapturedMetrics,
    /// Indicates whether this is a heartbeat metric report or
    /// session metric report (with session name)
    report_type: MetricReportType,
    /// Metric keys that should report a min and max value when
    /// aggregated as Histograms
    histo_min_max_metrics: MetricsSet,
}

struct MetricReportSnapshot {
    duration: Duration,
    boottime_duration: Option<Duration>,
    metrics: HashMap<MetricStringKey, MetricValue>,
}

impl MetricReport {
    pub fn new(report_type: MetricReportType, captured_metrics: CapturedMetrics) -> Self {
        Self {
            metrics: HashMap::new(),
            start: Instant::now(),
            boottime_start: get_system_clock(crate::util::system::Clock::Boottime).ok(),
            captured_metrics,
            report_type,
            histo_min_max_metrics: histo_min_max_keys(),
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
        let time_since_boot = get_system_clock(crate::util::system::Clock::Boottime).ok();
        let boottime_duration = match (self.boottime_start, time_since_boot) {
            (Some(boottime_start), Some(boottime_end)) => Some(boottime_end - boottime_start),
            _ => None,
        };
        let metrics = std::mem::take(&mut self.metrics)
            .into_iter()
            .flat_map(|(name, state)| match state.value() {
                MetricValue::Histogram(histo) => {
                    if self.histo_min_max_metrics.contains(&name) {
                        vec![
                            (name.with_suffix("_max"), histo.max()),
                            (name.with_suffix("_min"), histo.min()),
                            (name, histo.avg()),
                        ]
                    } else {
                        vec![(name, histo.avg())]
                    }
                }
                _ => vec![(name, state.value())],
            })
            .collect();

        MetricReportSnapshot {
            duration,
            boottime_duration,
            metrics,
        }
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
                snapshot.boottime_duration,
                self.report_type.clone(),
            ),
        )))
    }

    fn select_aggregate_for(event: &MetricReading) -> Result<Box<dyn TimeSeries + Send>> {
        match event {
            MetricReading::Histogram { .. } => Ok(Box::new(Histogram::new(event)?)),
            MetricReading::Counter { .. } => Ok(Box::new(Counter::new(event)?)),
            MetricReading::Gauge { .. } => Ok(Box::new(Gauge::new(event)?)),
            MetricReading::Rssi { .. } => Ok(Box::new(RssiAverage::new(event)?)),
            MetricReading::TimeWeightedAverage { .. } => {
                Ok(Box::new(TimeWeightedAverage::new(event)?))
            }
            MetricReading::ReportTag { .. } => Ok(Box::new(ReportTag::new(event)?)),
            MetricReading::Bool { .. } => Ok(Box::new(Bool::new(event)?)),
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
    use crate::{
        metrics::core_metrics::CoreMetricKeys,
        test_utils::{in_bools, in_counters, in_histograms},
    };
    use std::str::FromStr;

    use insta::assert_json_snapshot;
    use rstest::rstest;

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz",  3.0)]), "heartbeat_report_1")]
    // cpu_usage_memfaultd_pct should report a min and max as it matches a wildcard pattern
    // in the MetricsSet returned by MetricReport::histo_min_max_metrics
    #[case(in_histograms(vec![("cpu_usage_memfaultd_pct", 1.0), ("cpu_usage_memfaultd_pct", 2.0), ("cpu_usage_memfaultd_pct",  3.0)]), "heartbeat_report_2")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 1.0)]), "heartbeat_report_3")]
    #[case(in_histograms(vec![("memory_pct", 1.0), ("memory_pct", 2.0)]), "heartbeat_report_4")]
    #[case(in_histograms(vec![("memory_systemd_pct", 1.0), ("memory_systemd_pct", 2.0), ("memory_systemd_pct",  2.0)]), "heartbeat_report_5")]
    // bytes_per_second/rx should report a min and max, packets_per_second/rx should *not*
    #[case(in_histograms(vec![("interface/eth0/bytes_per_second/rx", 1.0), ("interface/eth0/bytes_per_second/rx", 2.0), ("interface/eth0/bytes_per_second/rx",  2.0)]), "heartbeat_report_6")]
    #[case(in_histograms(vec![("interface/eth0/packets_per_second/rx", 1.0), ("interface/eth0/packets_per_second/rx", 2.0), ("interface/eth0/packets_per_second/rx",  2.0)]), "heartbeat_report_7")]
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
    #[case(in_counters(vec![("operational_crashes", 2.0), ("operational_crashes_memfaultd", 3.0), ("operational_crashes_memfaultd",  2.0), ("crashes_systemd",  3.0)]), "session_report_6")]
    fn test_aggregate_metrics_session(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let session_core_metrics = CoreMetricKeys::get_session_core_metrics();
        let mut metric_keys = vec![
            MetricStringKey::from_str("foo").unwrap(),
            MetricStringKey::from_str("baz").unwrap(),
        ];
        metric_keys.extend(session_core_metrics.string_keys);
        let mut metric_report = MetricReport::new(
            MetricReportType::Session(SessionName::from_str("foo_only").unwrap()),
            CapturedMetrics::Metrics(MetricsSet {
                metric_keys: HashSet::from_iter(metric_keys),
                wildcard_metric_keys: session_core_metrics.wildcard_pattern_keys,
            }),
        );

        for m in metrics {
            metric_report.add_metric(m).unwrap();
        }
        let sorted_metrics: BTreeMap<_, _> = metric_report.take_metrics().into_iter().collect();
        assert_json_snapshot!(test_name, sorted_metrics);
    }

    #[rstest]
    #[case(in_bools(vec![("foo", true), ("bar", true), ("foo", false)]), "overwrite_previous_reading")]
    fn test_boolean_metrics(
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
