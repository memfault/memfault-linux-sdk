//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::sleep,
    time::{Duration, Instant},
};

use log::warn;

use crate::network::NetworkConfig;

use super::{MetricReportManager, MetricReportType};

/// A runner that periodically dumps metrics to a MAR entry
///
/// This runner will periodically dump metrics of a given type to a MAR entry
/// in the background. The runner will sleep between dumps.
pub struct PeriodicMetricReportDumper {
    metric_report_manager: Arc<Mutex<MetricReportManager>>,
    net_config: NetworkConfig,
    report_interval: Duration,
    mar_staging_path: PathBuf,
    report_type: MetricReportType,
}

impl PeriodicMetricReportDumper {
    pub fn new(
        mar_staging_path: PathBuf,
        net_config: NetworkConfig,
        metric_report_manager: Arc<Mutex<MetricReportManager>>,
        report_interval: Duration,
        report_type: MetricReportType,
    ) -> Self {
        Self {
            metric_report_manager,
            net_config,
            report_interval,
            mar_staging_path,
            report_type,
        }
    }

    pub fn start(&self) {
        let mut next_report = Instant::now() + self.report_interval;
        loop {
            while Instant::now() < next_report {
                sleep(next_report - Instant::now());
            }

            self.run_once(&mut next_report);
        }
    }

    fn run_once(&self, next_report: &mut Instant) {
        *next_report += self.report_interval;
        if let Err(e) = MetricReportManager::dump_report_to_mar_entry(
            &self.metric_report_manager,
            &self.mar_staging_path,
            &self.net_config,
            &self.report_type,
        ) {
            warn!("Unable to dump metrics: {}", e);
        }
    }
}

#[cfg(test)]
mod test {
    use tempfile::tempdir;

    use super::*;

    use crate::mar::manifest::Metadata;
    use crate::test_utils::in_histograms;

    #[test]
    fn test_happy_path_metric_report() {
        let report_manager = Arc::new(Mutex::new(MetricReportManager::new()));
        let tempdir = tempdir().unwrap();
        let mar_staging_path = tempdir.path().to_owned();
        let readings = in_histograms(vec![("hello", 10.0), ("mad", 20.0)]).collect::<Vec<_>>();

        {
            let mut report_manager = report_manager.lock().unwrap();

            for reading in &readings {
                report_manager.add_metric(reading.clone()).unwrap();
            }
        }

        let report_interval = Duration::from_secs(1);
        let runner = PeriodicMetricReportDumper::new(
            mar_staging_path.clone(),
            NetworkConfig::test_fixture(),
            report_manager,
            report_interval,
            MetricReportType::Heartbeat,
        );
        let mut next_report = Instant::now();
        let report_start = next_report;
        runner.run_once(&mut next_report);

        let entries = crate::mar::MarEntry::iterate_from_container(&mar_staging_path)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        assert_eq!(next_report.duration_since(report_start), report_interval);
        assert_eq!(entries.len(), 1);
        let metadata = &entries[0].manifest.metadata;
        match metadata {
            Metadata::LinuxMetricReport {
                metrics,
                report_type,
                ..
            } => {
                assert_eq!(metrics.len(), 2);

                matches!(report_type, &MetricReportType::Heartbeat);

                for reading in readings {
                    assert!(metrics.contains_key(&reading.name));
                }
            }
            _ => panic!("Unexpected metadata"),
        }
    }
}
