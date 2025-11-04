//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use log::{debug, error, warn};
use ssf::{Handler, Service};
use std::mem::replace;
use std::num::NonZeroU32;
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
};

use crate::{
    config::SessionConfig,
    mar::{MarConfig, MarEntryBuilder, Metadata},
    metrics::{
        core_metrics::{CoreMetricKeys, METRIC_OPERATIONAL_CRASHES},
        hrt::HrtReport,
        metric_reading::KeyedMetricReading,
        metric_report::{CapturedMetrics, MetricsSet},
        MetricReport, MetricReportType, MetricStringKey, MetricValue, SessionEventMessage,
        SessionName,
    },
    network::NetworkConfig,
};

use super::hrt::HRT_DEFAULT_MAX_SAMPLES_PER_MIN;
use super::{hrt::write_report_to_disk, DumpHrtMessage, DumpMetricReportMessage, ReportsToDump};

pub struct MetricReportManager {
    heartbeat: MetricReport,
    daily_heartbeat: Option<MetricReport>,
    hrt: Option<HrtReport>,
    sessions: HashMap<SessionName, MetricReport>,
    session_configs: Vec<SessionConfig>,
    core_metrics: CoreMetricKeys,
    hrt_max_samples_per_min: NonZeroU32,
}

impl MetricReportManager {
    /// Creates a MetricReportManager with no sessions
    /// configured
    pub fn new(
        hrt_enabled: bool,
        hrt_max_samples_per_min: NonZeroU32,
        daily_heartbeats_enabled: bool,
    ) -> Self {
        Self::new_with_session_configs(
            hrt_enabled,
            hrt_max_samples_per_min,
            &[],
            daily_heartbeats_enabled,
        )
    }

    pub fn new_with_session_configs(
        hrt_enabled: bool,
        hrt_max_samples_per_min: NonZeroU32,
        session_configs: &[SessionConfig],
        daily_heartbeats_enabled: bool,
    ) -> Self {
        Self {
            heartbeat: MetricReport::new_heartbeat(),
            daily_heartbeat: daily_heartbeats_enabled.then(MetricReport::new_daily_heartbeat),
            hrt: hrt_enabled.then(|| HrtReport::new(hrt_max_samples_per_min)),
            sessions: HashMap::new(),
            session_configs: session_configs.to_vec(),
            core_metrics: CoreMetricKeys::get_session_core_metrics(),
            hrt_max_samples_per_min,
        }
    }

    /// Starts a session of the specified session name.
    /// Fails if the session name provided is not configured.
    /// If there is already a session with that name ongoing,
    /// this is a no-op
    pub fn start_session(&mut self, session_name: SessionName) -> Result<()> {
        let report_type = MetricReportType::Session(session_name.clone());
        let captured_metric_keys = self.captured_metric_keys_for_report(&report_type)?;

        if let Entry::Vacant(e) = self.sessions.entry(session_name) {
            let session = e.insert(MetricReport::new(report_type, captured_metric_keys));
            // Make sure we always include the operational_crashes counter in every session report.
            session.add_to_counter(METRIC_OPERATIONAL_CRASHES, 0.0)?;
        }
        Ok(())
    }

    /// Returns the metrics the provided session name is configured to capture
    fn captured_metric_keys_for_report(
        &self,
        report_type: &MetricReportType,
    ) -> Result<CapturedMetrics> {
        match report_type {
            MetricReportType::Heartbeat => Ok(CapturedMetrics::All),
            MetricReportType::DailyHeartbeat => Ok(CapturedMetrics::All),
            MetricReportType::Session(session_name) => {
                let mut metrics = self
                    .session_configs
                    .iter()
                    .find(|&session_config| session_config.name == *session_name)
                    .map(|config| config.captured_metrics.clone())
                    .ok_or_else(|| {
                        eyre!("No configuration for session named {} found!", session_name)
                    })?;

                metrics.extend(self.core_metrics.string_keys.clone());

                Ok(CapturedMetrics::Metrics(MetricsSet {
                    metric_keys: metrics,
                    wildcard_metric_keys: self.core_metrics.wildcard_pattern_keys.clone(),
                }))
            }
        }
    }

    /// Returns an iterator over all ongoing metric reports
    fn report_iter(&mut self) -> impl Iterator<Item = &mut MetricReport> {
        let reports = match &mut self.daily_heartbeat {
            Some(daily_heartbeat) => vec![&mut self.heartbeat, daily_heartbeat],
            None => vec![&mut self.heartbeat],
        };
        self.sessions.values_mut().chain(reports)
    }

    /// Adds a metric reading to all ongoing metric reports
    /// that capture that metric
    pub fn add_metric(&mut self, m: KeyedMetricReading) -> Result<()> {
        if let Some(hrt_report) = &mut self.hrt {
            hrt_report.add_metric(&m);
        }
        self.report_iter()
            .try_for_each(|report| report.add_metric(m.clone()))
    }

    /// Increment a counter metric by 1
    pub fn increment_counter(&mut self, name: &str) -> Result<()> {
        self.add_metric(KeyedMetricReading::new_counter(
            MetricStringKey::from_str(name)
                .map_err(|e| eyre!("Couldn't construct metric key: {}", e))?,
            1.0,
        ))
    }

    /// Increment a counter by a specified amount
    pub fn add_to_counter(&mut self, name: &str, value: f64) -> Result<()> {
        self.add_metric(KeyedMetricReading::new_counter(
            MetricStringKey::from_str(name)
                .map_err(|e| eyre!("Couldn't construct metric key: {}", e))?,
            value,
        ))
    }

    /// Adds a metric reading to a specific metric report
    pub fn add_metric_to_report(
        &mut self,
        report_type: &MetricReportType,
        m: KeyedMetricReading,
    ) -> Result<()> {
        match report_type {
            MetricReportType::Heartbeat => self.heartbeat.add_metric(m),
            MetricReportType::DailyHeartbeat => {
                if let Some(daily_heartbeat) = &mut self.daily_heartbeat {
                    daily_heartbeat.add_metric(m)
                } else {
                    Ok(())
                }
            }
            MetricReportType::Session(session_name) => self
                .sessions
                .get_mut(session_name)
                .ok_or_else(|| eyre!("No ongoing session with name {}", session_name))
                .and_then(|session_report| session_report.add_metric(m)),
        }
    }

    /// Return all the metrics in memory and resets the
    /// store for the periodic heartbeat report.
    pub fn take_heartbeat_metrics(&mut self) -> HashMap<MetricStringKey, MetricValue> {
        self.heartbeat.take_metrics()
    }

    /// Return all the metrics in memory and resets the store
    /// for a specified session.
    pub fn take_session_metrics(
        &mut self,
        session_name: &SessionName,
    ) -> Result<HashMap<MetricStringKey, MetricValue>> {
        self.sessions
            .get_mut(session_name)
            .ok_or_else(|| eyre!("No ongoing session with name {}", session_name))
            .map(|session_report| session_report.take_metrics())
    }

    /// Dump the metrics to a MAR entry.
    ///
    /// This will empty the metrics store.
    /// When used with a heartbeat metric report type, the heartbeat
    /// will be reset.
    /// When used with a session report type, the session will end and
    /// be removed from the MetricReportManager's internal sessions HashMap.
    pub fn dump_report_to_mar_entry(
        &mut self,
        network_config: &NetworkConfig,
        report_type: &MetricReportType,
        mar_config: &MarConfig,
    ) -> Result<()> {
        let mar_staging_area = mar_config.tmp_staging_path();
        let mar_builder = match report_type {
            MetricReportType::Heartbeat => {
                self.heartbeat.prepare_metric_report(&mar_staging_area)?
            }
            MetricReportType::DailyHeartbeat => match &mut self.daily_heartbeat {
                Some(daily_heartbeat) => {
                    daily_heartbeat.prepare_metric_report(&mar_staging_area)?
                }
                None => return Ok(()),
            },
            MetricReportType::Session(session_name) => match self.sessions.remove(session_name) {
                Some(mut report) => report.prepare_metric_report(&mar_staging_area)?,
                None => return Err(eyre!("No metric report found for {}", session_name)),
            },
        };

        if let Some(mar_builder) = mar_builder {
            let mar_entry = mar_builder
                .save(network_config, mar_config)
                .map_err(|e| eyre!("Error building MAR entry: {}", e))?;
            debug!(
                "Generated MAR entry from metrics: {}",
                mar_entry.path.display()
            );
        } else {
            debug!(
                "Skipping generating metrics entry. No metrics in store for: {}",
                report_type.as_str()
            )
        }
        Ok(())
    }

    fn prepare_all_metric_reports(
        &mut self,
        mar_staging_area: &Path,
    ) -> Vec<MarEntryBuilder<Metadata>> {
        self.report_iter()
            .filter_map(|report| {
                if let Ok(builder) = report.prepare_metric_report(mar_staging_area) {
                    builder.or_else(|| {
                        debug!(
                            "Skipping generating metrics entry. No metrics in store for: {}",
                            report.report_type().as_str()
                        );
                        None
                    })
                } else {
                    debug!(
                        "Failed to prepare metric report for: {}",
                        report.report_type().as_str()
                    );
                    None
                }
            })
            .collect()
    }

    /// Ends all ongoing MetricReports and dumps them as MARs to disk.
    ///
    /// MetricReports with the MetricReportType specified with
    /// exclude_report_types are excluded from this operation
    /// entirely
    pub fn dump_metric_reports(
        metric_report_manager: &Arc<Mutex<Self>>,
        mar_staging_area: &Path,
        network_config: &NetworkConfig,
        mar_config: &MarConfig,
    ) -> Result<()> {
        let mar_builders = metric_report_manager
            .lock()
            .expect("Mutex poisoned")
            .prepare_all_metric_reports(mar_staging_area);

        for mar_builder in mar_builders {
            match mar_builder.save(network_config, mar_config) {
                Ok(mar_entry) => debug!(
                    "Generated MAR entry from metrics: {}",
                    mar_entry.path.display()
                ),
                Err(e) => error!("Error building MAR entry: {}", e),
            }
        }

        Ok(())
    }
}

impl Default for MetricReportManager {
    fn default() -> Self {
        Self::new(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Default HRT rate limit should be nonzero"),
            true,
        )
    }
}

impl Service for MetricReportManager {
    fn name(&self) -> &str {
        "MetricReportManager"
    }
}

impl Handler<KeyedMetricReading> for MetricReportManager {
    fn deliver(&mut self, m: KeyedMetricReading) -> Result<()> {
        self.add_metric(m)
    }
}

impl Handler<DumpMetricReportMessage> for MetricReportManager {
    fn deliver(&mut self, m: DumpMetricReportMessage) -> Result<()> {
        let network_config = m.network_config();
        let mar_config = m.mar_config();

        match m.reports_to_dump() {
            ReportsToDump::Report(report_type) => {
                if let Err(e) =
                    self.dump_report_to_mar_entry(network_config, report_type, mar_config)
                {
                    warn!("Failed to dump {:?} metric report: {}", report_type, e)
                }
            }
            ReportsToDump::All => {
                let mar_builders = self.prepare_all_metric_reports(&mar_config.tmp_staging_path());
                for mar_builder in mar_builders {
                    match mar_builder.save(network_config, mar_config) {
                        Ok(mar_entry) => debug!(
                            "Generated MAR entry from metrics: {}",
                            mar_entry.path.display()
                        ),
                        Err(e) => warn!("Error building MAR entry: {}", e),
                    }
                }
            }
        }

        Ok(())
    }
}

impl Handler<SessionEventMessage> for MetricReportManager {
    fn deliver(&mut self, m: SessionEventMessage) -> <SessionEventMessage as ssf::Message>::Reply {
        match m {
            SessionEventMessage::StartSession { name, readings } => {
                let report = MetricReportType::Session(name.clone());
                self.start_session(name)?;
                for metric_reading in readings {
                    self.add_metric_to_report(&report, metric_reading)?
                }
            }
            SessionEventMessage::StopSession {
                name,
                readings,
                network_config,
                mar_config,
            } => {
                let report = MetricReportType::Session(name);
                for metric_reading in readings {
                    self.add_metric_to_report(&report, metric_reading)?
                }

                self.dump_report_to_mar_entry(&network_config, &report, &mar_config)?;
            }
        };
        Ok(())
    }
}

impl Handler<DumpHrtMessage> for MetricReportManager {
    fn deliver(&mut self, m: DumpHrtMessage) -> <DumpHrtMessage as ssf::Message>::Reply {
        if let Some(hrt) = &mut self.hrt {
            // Replace the HRT report with a new one and write it to disk
            let hrt_report = replace(hrt, HrtReport::new(self.hrt_max_samples_per_min));
            write_report_to_disk(hrt_report, m.network_config(), m.mar_config())?;
        }
        Ok(())
    }
}

/// A trait to make it easier to verify (in unit tests) all the code that pushes metrics.
///
/// The implementation should implement some logic to coalesce multiple metrics
/// of the same name into one value.
pub trait TakeMetrics {
    fn take_metrics(&mut self) -> Result<BTreeMap<MetricStringKey, MetricValue>>;
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{metrics::hrt::HighResTelemetryV1, test_utils::in_histograms};
    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;
    use ssf::ServiceMock;
    use std::{collections::HashSet, str::FromStr};

    impl TakeMetrics for ServiceMock<Vec<KeyedMetricReading>> {
        fn take_metrics(&mut self) -> Result<BTreeMap<MetricStringKey, MetricValue>> {
            let mut metric_service = MetricReportManager::default();
            for m in self.take_messages().into_iter().flatten() {
                metric_service.deliver(m)?;
            }
            Ok(metric_service
                .take_heartbeat_metrics()
                .into_iter()
                .collect())
        }
    }

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar",  2.0), ("baz", 3.0)]), "heartbeat_report_1")]
    #[case(in_histograms(vec![("foo",  1.0), ("foo",2.0), ("foo", 3.0)]), "heartbeat_report_2")]
    #[case(in_histograms(vec![("foo",  1.0), ("foo",1.0)]), "heartbeat_report_3")]
    #[case(in_histograms(vec![("foo",  1.0), ("foo",2.0)]), "heartbeat_report_4")]
    #[case(in_histograms(vec![("foo",  1.0), ("foo",2.0), ("foo", 2.0)]), "heartbeat_report_5")]
    fn test_heartbeat_report(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let mut metric_report_manager = MetricReportManager::default();
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
        assert_json_snapshot!(test_name, builder.unwrap().get_metadata(), {".metadata.duration_ms" => 0, ".metadata.boottime_duration_ms" => 0});
    }

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar",  2.0), ("baz", 3.0)]))]
    fn test_no_hrt_when_disabled(#[case] metrics: impl Iterator<Item = KeyedMetricReading>) {
        let mut metric_report_manager =
            MetricReportManager::new(false, NonZeroU32::new(1).unwrap(), true);
        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        assert!(metric_report_manager.hrt.is_none());
    }

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar",  2.0), ("baz", 3.0)]))]
    fn test_hrt_when_enabled(#[case] metrics: impl Iterator<Item = KeyedMetricReading>) {
        let mut metric_report_manager = MetricReportManager::new(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Default HRT rate limit should be nonzero"),
            true,
        );
        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        assert!(metric_report_manager.hrt.is_some());
        let mut hrt_report_serialized =
            HighResTelemetryV1::try_from(metric_report_manager.hrt.unwrap()).unwrap();
        hrt_report_serialized.sort_rollups();
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(hrt_report_serialized,
                                  {".producer.version" => "[version]", ".start_time" => "[start_time]", ".rollups[].data[].t" => "[timestamp]", ".duration_ms" => "[duration]", ".boottime_duration_ms" => "[duration]"});
        });
    }
    #[rstest]
    fn test_unconfigured_session_name_fails() {
        let mut metric_report_manager = MetricReportManager::default();
        assert!(metric_report_manager
            .start_session(SessionName::from_str("test-session").unwrap())
            .is_err())
    }

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz",  3.0)]), "heartbeat_and_sessions_report_1")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0), ("foo",  3.0)]), "heartbeat_and_sessions_report_2")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 1.0)]), "heartbeat_and_sessions_report_3")]
    #[case(in_histograms(vec![("foo", 1.0), ("foo", 2.0), ("baz", 1.0), ("baz",  2.0)]), "heartbeat_and_sessions_report_4")]
    #[case(in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("foo", 2.0)]), "heartbeat_and_sessions_report_5")]
    fn test_heartbeat_and_session_reports(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let session_a_name = SessionName::from_str("test-session-some-metrics").unwrap();
        let session_b_name = SessionName::from_str("test-session-all-metrics").unwrap();
        let session_configs = vec![
            SessionConfig {
                name: session_a_name.clone(),
                captured_metrics: HashSet::from_iter([
                    MetricStringKey::from_str("foo").unwrap(),
                    MetricStringKey::from_str("bar").unwrap(),
                ]),
            },
            SessionConfig {
                name: session_b_name.clone(),
                captured_metrics: HashSet::from_iter([
                    MetricStringKey::from_str("foo").unwrap(),
                    MetricStringKey::from_str("bar").unwrap(),
                    MetricStringKey::from_str("baz").unwrap(),
                ]),
            },
        ];

        let mut metric_report_manager = MetricReportManager::new_with_session_configs(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Zero value passed to non-zero constructor"),
            &session_configs,
            true,
        );

        assert!(metric_report_manager.start_session(session_a_name).is_ok());
        assert!(metric_report_manager.start_session(session_b_name).is_ok());

        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        let tempdir = TempDir::new().unwrap();
        // Verify heartbeat report
        let snapshot_name = format!("{}.{}", test_name, "heartbeat");
        assert_report_snapshot(
            &mut metric_report_manager.heartbeat,
            &snapshot_name,
            &tempdir,
        );

        // Verify daily heartbeat report
        let snapshot_name = format!("{}.{}", test_name, "daily_heartbeat");
        assert_report_snapshot(
            &mut metric_report_manager.daily_heartbeat.unwrap(),
            &snapshot_name,
            &tempdir,
        );

        for (session_name, mut metric_report) in metric_report_manager.sessions {
            let snapshot_name = format!("{}.{}", test_name, session_name);
            assert_report_snapshot(&mut metric_report, &snapshot_name, &tempdir);
        }
    }

    #[rstest]
    #[case(in_histograms(vec![("foo", 1.0), ("cpu_usage_memfaultd_pct", 2.0), ("memory_pct",  3.0)]), "system_and_process_metrics")]
    #[case(in_histograms(vec![("memory_systemd_pct", 1.0), ("memory_memfaultd_pct", 2.0), ("memory_foo_pct", 2.0)]), "process_metrics")]
    fn test_sessions_capture_core_metrics(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] test_name: &str,
    ) {
        let session_name = SessionName::from_str("test-session").unwrap();
        let session_configs = vec![SessionConfig {
            name: session_name.clone(),
            captured_metrics: HashSet::from_iter([
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
            ]),
        }];

        let mut metric_report_manager = MetricReportManager::new_with_session_configs(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Zero value passed to non-zero constructor"),
            &session_configs,
            true,
        );

        assert!(metric_report_manager
            .start_session(session_name.clone())
            .is_ok());

        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        let session_report = metric_report_manager
            .sessions
            .get_mut(&session_name)
            .unwrap();
        let metrics = session_report.take_metrics();

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "metrics"),
                                  metrics,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }

    #[rstest]
    fn test_start_session_twice() {
        let session_name = SessionName::from_str("test-session-start-twice").unwrap();
        let session_configs = vec![SessionConfig {
            name: session_name.clone(),
            captured_metrics: HashSet::from_iter([
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
            ]),
        }];

        let mut metric_report_manager = MetricReportManager::new_with_session_configs(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Zero value passed to non-zero constructor"),
            &session_configs,
            true,
        );

        let metrics_a = in_histograms(vec![("foo", 1.0), ("bar", 2.0)]);
        assert!(metric_report_manager
            .start_session(session_name.clone())
            .is_ok());
        for m in metrics_a {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        // Final metric report should aggregate both metrics_a and
        // metrics_b as the session should not be restarted
        // by the second start_session
        let metrics_b = in_histograms(vec![("foo", 9.0), ("bar", 5.0)]);
        assert!(metric_report_manager
            .start_session(session_name.clone())
            .is_ok());
        for m in metrics_b {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        let tempdir = TempDir::new().unwrap();
        let builder = metric_report_manager
            .sessions
            .get_mut(&session_name)
            .unwrap()
            .prepare_metric_report(tempdir.path())
            .unwrap();

        assert_json_snapshot!(builder.unwrap().get_metadata(), {".metadata.duration_ms" => 0, ".metadata.boottime_duration_ms" => 0});
    }

    #[rstest]
    fn test_prepare_all_prepares_sessions() {
        let session_name = SessionName::from_str("test-session").unwrap();
        let session_configs = vec![SessionConfig {
            name: session_name.clone(),
            captured_metrics: HashSet::from_iter([
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
            ]),
        }];

        let mut metric_report_manager = MetricReportManager::new_with_session_configs(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Zero value passed to non-zero constructor"),
            &session_configs,
            true,
        );

        let metrics = in_histograms(vec![("foo", 5.0), ("bar", 3.5)]);
        assert!(metric_report_manager.start_session(session_name).is_ok());
        for m in metrics {
            metric_report_manager
                .add_metric(m)
                .expect("Failed to add metric reading");
        }

        let tempdir = TempDir::new().unwrap();
        let builders = metric_report_manager.prepare_all_metric_reports(tempdir.path());

        // 3 MAR builders should be created for "heartbeat", "daily-heartbeat", and "test-session"
        // Note this only works because report_iter() with only 1 session is deterministic
        for builder in builders {
            match builder.get_metadata() {
                Metadata::LinuxMetricReport { report_type, .. } => {
                    assert_json_snapshot!(report_type.as_str(), builder.get_metadata(), {
                        ".metadata.duration_ms" => 0,
                        ".metadata.boottime_duration_ms" => 0
                    })
                }
                _ => panic!("Invalid MAR builder"),
            }
        }
    }

    #[test]
    fn test_no_daily_heartbeats_when_disabled() {
        let metric_report_manager = MetricReportManager::new(
            true,
            NonZeroU32::new(HRT_DEFAULT_MAX_SAMPLES_PER_MIN)
                .expect("Zero value passed to non-zero constructor"),
            false,
        );

        assert!(metric_report_manager.daily_heartbeat.is_none());
    }

    fn assert_report_snapshot(
        metric_report: &mut MetricReport,
        snapshot_name: &str,
        tempdir: &TempDir,
    ) {
        let builder = metric_report.prepare_metric_report(tempdir.path()).unwrap();
        assert_json_snapshot!(snapshot_name, builder.unwrap().get_metadata(), {".metadata.duration_ms" => 0, ".metadata.boottime_duration_ms" => 0});
    }
}
