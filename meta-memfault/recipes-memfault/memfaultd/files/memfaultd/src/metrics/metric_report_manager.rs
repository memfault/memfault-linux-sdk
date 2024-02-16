//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use log::debug;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use super::{metric_reading::KeyedMetricReading, metric_report::CapturedMetrics, SessionName};
use crate::{
    config::SessionConfig,
    metrics::{MetricReport, MetricReportType, MetricStringKey, MetricValue},
    network::NetworkConfig,
};

pub struct MetricReportManager {
    heartbeat: MetricReport,
    sessions: HashMap<SessionName, MetricReport>,
    session_configs: Vec<SessionConfig>,
}

impl MetricReportManager {
    /// Creates a MetricReportManager with no sessions
    /// configured
    pub fn new() -> Self {
        Self {
            heartbeat: MetricReport::new_heartbeat(),
            sessions: HashMap::new(),
            session_configs: vec![],
        }
    }

    pub fn new_with_session_configs(session_configs: &[SessionConfig]) -> Self {
        Self {
            heartbeat: MetricReport::new_heartbeat(),
            sessions: HashMap::new(),
            session_configs: session_configs.to_vec(),
        }
    }

    /// Starts a session of the specified session name.
    /// Fails if the session name provided is not configured.
    /// If there is already a session with that name ongoing,
    /// the ongoing session will be dropped and a fresh one will be
    /// created. The dropped session is *not* written to disk.
    pub fn start_session(&mut self, session_name: SessionName) -> Result<()> {
        let report_type = MetricReportType::Session(session_name.clone());
        let captured_metric_keys = self.captured_metric_keys_for_report(&report_type)?;

        self.sessions.insert(
            session_name,
            MetricReport::new(report_type, captured_metric_keys),
        );
        Ok(())
    }

    /// Returns the metrics the provided session name is configured to capture
    fn captured_metric_keys_for_report(
        &self,
        report_type: &MetricReportType,
    ) -> Result<CapturedMetrics> {
        match report_type {
            MetricReportType::Heartbeat => Ok(CapturedMetrics::All),
            MetricReportType::Session(session_name) => self
                .session_configs
                .iter()
                .find(|&session_config| session_config.name == *session_name)
                .map(|config| CapturedMetrics::Metrics(config.captured_metrics.clone()))
                .ok_or_else(|| eyre!("No configuration for session named {} found!", session_name)),
        }
    }

    /// Adds a metric reading to all ongoing metric reports
    /// that capture that metric
    pub fn add_metric(&mut self, m: KeyedMetricReading) -> Result<()> {
        self.heartbeat.add_metric(m.clone())?;
        for session_report in self.sessions.values_mut() {
            session_report.add_metric(m.clone())?
        }
        Ok(())
    }

    /// Increment a counter metric by 1
    pub fn increment_counter(&mut self, name: &str) -> Result<()> {
        self.heartbeat.increment_counter(name)?;
        for session_report in self.sessions.values_mut() {
            session_report.increment_counter(name)?
        }
        Ok(())
    }

    /// Increment a counter by a specified amount
    pub fn add_to_counter(&mut self, name: &str, value: f64) -> Result<()> {
        self.heartbeat.add_to_counter(name, value)?;
        for session_report in self.sessions.values_mut() {
            session_report.add_to_counter(name, value)?
        }
        Ok(())
    }

    /// Return all the metrics in memory and resets the store.
    pub fn take_heartbeat_metrics(&mut self) -> HashMap<MetricStringKey, MetricValue> {
        self.heartbeat.take_metrics()
    }

    /// Return all the metrics in memory and resets the store.
    pub fn take_session_metrics(
        &mut self,
        session_name: &SessionName,
    ) -> Result<HashMap<MetricStringKey, MetricValue>> {
        self.sessions
            .get_mut(session_name)
            .ok_or_else(|| eyre!("No ongoing session with name {}", session_name))
            .map(|session_report| session_report.take_metrics())
    }

    /// Dump the metrics to a MAR entry. This takes a
    /// &Arc<Mutex<MetricReportManager>> and will minimize lock time.
    /// This will empty the metrics store.
    /// When used with a heartbeat metric report type, the heartbeat
    /// will be reset.
    /// When used with a session report type, the session will end and
    /// be removed from the MetricReportManager's internal sessions HashMap.
    pub fn dump_report_to_mar_entry(
        metric_report_manager: &Arc<Mutex<Self>>,
        mar_staging_area: &Path,
        network_config: &NetworkConfig,
        report_type: MetricReportType,
    ) -> Result<()> {
        let mar_builder = match &report_type {
            MetricReportType::Heartbeat => metric_report_manager
                .lock()
                .expect("Mutex Poisoned!")
                .heartbeat
                .prepare_metric_report(mar_staging_area)?,
            MetricReportType::Session(session_name) => {
                match metric_report_manager
                    .lock()
                    .expect("Mutex Poisoned!")
                    .sessions
                    .remove(session_name)
                {
                    Some(mut report) => report.prepare_metric_report(mar_staging_area)?,
                    None => return Err(eyre!("No metric report found for {}", session_name)),
                }
            }
        };

        // Save to disk after releasing the lock
        if let Some(mar_builder) = mar_builder {
            let mar_entry = mar_builder
                .save(network_config)
                .map_err(|e| eyre!("Error building MAR entry: {}", e))?;
            debug!(
                "Generated MAR entry from metrics: {}",
                mar_entry.path.display()
            );
        } else {
            let report_name = match &report_type {
                MetricReportType::Heartbeat => "heartbeat",
                MetricReportType::Session(session_name) => session_name.as_str(),
            };
            debug!(
                "Skipping generating metrics entry. No metrics in store for: {}",
                report_name
            )
        }
        Ok(())
    }
}

impl Default for MetricReportManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::test_utils::in_gauges;
    use insta::assert_json_snapshot;
    use rstest::rstest;
    use std::str::FromStr;

    #[rstest]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("bar", 1000, 2.0), ("baz", 1000, 3.0)]), "heartbeat_report_1")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 3.0)]), "heartbeat_report_2")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 1.0)]), "heartbeat_report_3")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0)]), "heartbeat_report_4")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 2.0)]), "heartbeat_report_5")]
    fn test_heartbeat_report(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let mut metric_report_manager = MetricReportManager::new();
        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        let tempdir = TempDir::new().unwrap();
        let builder = metric_report_manager
            .heartbeat
            .prepare_metric_report(tempdir.path())
            .unwrap();
        assert_json_snapshot!(test_name, builder.unwrap().get_metadata());
    }

    #[rstest]
    fn test_unconfigured_session_name_fails() {
        let mut metric_report_manager = MetricReportManager::new();
        assert!(metric_report_manager
            .start_session(SessionName::from_str("test-session").unwrap())
            .is_err())
    }

    #[rstest]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("bar", 1000, 2.0), ("baz", 1000, 3.0)]), "heartbeat_and_sessions_report_1")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 3.0)]), "heartbeat_and_sessions_report_2")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 1.0)]), "heartbeat_and_sessions_report_3")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("baz", 1000, 1.0), ("baz", 1000, 2.0)]), "heartbeat_and_sessions_report_4")]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("bar", 1000, 2.0), ("foo", 1000, 2.0)]), "heartbeat_and_sessions_report_5")]
    fn test_heartbeat_and_session_reports(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let session_a_name = SessionName::from_str("test-session-some-metrics").unwrap();
        let session_b_name = SessionName::from_str("test-session-all-metrics").unwrap();
        let session_configs = vec![
            SessionConfig {
                name: session_a_name.clone(),
                captured_metrics: vec![
                    MetricStringKey::from_str("foo").unwrap(),
                    MetricStringKey::from_str("bar").unwrap(),
                ],
            },
            SessionConfig {
                name: session_b_name.clone(),
                captured_metrics: vec![
                    MetricStringKey::from_str("foo").unwrap(),
                    MetricStringKey::from_str("bar").unwrap(),
                    MetricStringKey::from_str("baz").unwrap(),
                ],
            },
        ];

        let mut metric_report_manager =
            MetricReportManager::new_with_session_configs(&session_configs);

        assert!(metric_report_manager.start_session(session_a_name).is_ok());
        assert!(metric_report_manager.start_session(session_b_name).is_ok());

        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        let tempdir = TempDir::new().unwrap();
        let builder = metric_report_manager
            .heartbeat
            .prepare_metric_report(tempdir.path())
            .unwrap();

        let snapshot_name = format!("{}.{}", test_name, "heartbeat");
        assert_json_snapshot!(snapshot_name, builder.unwrap().get_metadata(), {".metadata.duration_ms" => 0});

        for (session_name, mut metric_report) in metric_report_manager.sessions {
            let builder = metric_report.prepare_metric_report(tempdir.path()).unwrap();
            let snapshot_name = format!("{}.{}", test_name, session_name);
            assert_json_snapshot!(snapshot_name, builder.unwrap().get_metadata(), {".metadata.duration_ms" => 0});
        }
    }
}
