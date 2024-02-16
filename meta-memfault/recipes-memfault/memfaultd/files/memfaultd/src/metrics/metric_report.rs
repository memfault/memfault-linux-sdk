//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Utc;
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

use crate::{
    mar::{MarEntryBuilder, Metadata},
    metrics::{MetricReading, MetricStringKey, MetricValue, SessionName},
};

use super::{
    battery::METRIC_BATTERY_SOC_PCT,
    metric_reading::KeyedMetricReading,
    timeseries::{Counter, Histogram, TimeSeries, TimeWeightedAverage},
};

pub enum CapturedMetrics {
    All,
    Metrics(Vec<MetricStringKey>),
}

#[derive(Serialize, Deserialize, Clone)]
pub enum MetricReportType {
    #[serde(rename = "heartbeat")]
    Heartbeat,
    #[serde(rename = "session")]
    Session(SessionName),
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
    pub fn new(report_type: MetricReportType, captured_metrics: CapturedMetrics) -> Self {
        Self {
            metrics: HashMap::new(),
            start: Instant::now(),
            captured_metrics,
            report_type,
        }
    }

    // Creates a heartbeat report that captures all metrics
    pub fn new_heartbeat() -> Self {
        MetricReport::new(MetricReportType::Heartbeat, CapturedMetrics::All)
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
                    let key = o.key().clone();
                    let state = o.get_mut();
                    if let Err(e) = (*state).aggregate(&m.value) {
                        *state = Self::select_aggregate_for(&key, &m.value)?;
                        log::warn!(
                            "New value for metric {} is incompatible ({}). Resetting timeseries.",
                            o.key(),
                            e
                        );
                    }
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    let timeseries = Self::select_aggregate_for(v.key(), &m.value)?;
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
            Metadata::LinuxMetricReport {
                metrics: snapshot.metrics,
                duration: snapshot.duration,
                report_type: self.report_type.clone(),
            },
        )))
    }

    fn select_aggregate_for(
        key: &MetricStringKey,
        event: &MetricReading,
    ) -> Result<Box<dyn TimeSeries + Send>> {
        match event {
            MetricReading::Gauge { .. } => {
                // Very basic heuristic for now - in the future we may want to make this user-configurable.
                if key.as_str().eq(METRIC_BATTERY_SOC_PCT) {
                    Ok(Box::new(TimeWeightedAverage::new(event)?))
                } else {
                    Ok(Box::new(Histogram::new(event)?))
                }
            }
            MetricReading::Counter { .. } => Ok(Box::new(Counter::new(event)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use std::collections::BTreeMap;

    use super::*;
    use crate::metrics::MetricTimestamp;
    use chrono::Duration;
    use std::str::FromStr;

    use insta::assert_json_snapshot;
    use rstest::rstest;

    #[rstest]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("bar", 1000, 2.0), ("baz", 1000, 3.0)]), "heartbeat_report_1")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 3.0)]), "heartbeat_report_2")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 1.0)]), "heartbeat_report_3")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0)]), "heartbeat_report_4")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 2.0)]), "heartbeat_report_5")]
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
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("bar", 1000, 2.0), ("baz", 1000, 3.0)]), "sesion_report_1")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 3.0)]), "sesion_report_2")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 1.0)]), "sesion_report_3")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0)]), "sesion_report_4")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("baz", 1000, 2.0), ("bar", 1000, 3.0)]), "sesion_report_5")]
    fn test_aggregate_metrics_session(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let mut metric_report = MetricReport::new(
            MetricReportType::Session(SessionName::from_str("foo_only").unwrap()),
            CapturedMetrics::Metrics(vec![
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("baz").unwrap(),
            ]),
        );

        for m in metrics {
            metric_report.add_metric(m).unwrap();
        }
        let sorted_metrics: BTreeMap<_, _> = metric_report.take_metrics().into_iter().collect();
        assert_json_snapshot!(test_name, sorted_metrics);
    }

    #[rstest]
    fn test_empty_after_write() {
        let mut metric_report = MetricReport::new_heartbeat();
        for m in in_gauges(vec![
            ("foo", 1000, 1.0),
            ("bar", 1000, 2.0),
            ("baz", 1000, 3.0),
        ]) {
            metric_report.add_metric(m).unwrap();
        }

        let tempdir = TempDir::new().unwrap();
        let _ = metric_report.prepare_metric_report(tempdir.path());
        assert_eq!(metric_report.take_metrics().len(), 0);
    }

    fn in_gauges(
        metrics: Vec<(&'static str, i64, f64)>,
    ) -> impl Iterator<Item = KeyedMetricReading> {
        metrics
            .into_iter()
            .enumerate()
            .map(|(i, (name, interval, value))| KeyedMetricReading {
                name: MetricStringKey::from_str(name).unwrap(),
                value: MetricReading::Gauge {
                    value,
                    interval: Duration::milliseconds(interval),
                    timestamp: MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap()
                        + chrono::Duration::seconds(i as i64),
                },
            })
    }
}
