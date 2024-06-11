//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::thread::spawn;

use eyre::Result;
use log::warn;

use crate::metrics::{KeyedMetricReading, MetricReportManager};

pub struct StatsDServer {}

impl StatsDServer {
    pub fn new() -> StatsDServer {
        StatsDServer {}
    }

    pub fn start(
        &self,
        listening_address: SocketAddr,
        metric_report_manager: Arc<Mutex<MetricReportManager>>,
    ) -> Result<()> {
        let socket = UdpSocket::bind(listening_address)?;
        spawn(move || {
            loop {
                // This means that packets with > 1432 bytes are NOT supported
                // Clients must enforce a maximum message size of 1432 bytes or less
                let mut buf = [0; 1432];
                match socket.recv(&mut buf) {
                    Ok(amt) => {
                        let message = String::from_utf8_lossy(&buf[..amt]);
                        Self::process_statsd_message(&message, &metric_report_manager)
                    }
                    Err(e) => warn!("Statsd server socket error: {}", e),
                }
            }
        });
        Ok(())
    }

    fn process_statsd_message(
        message: &str,
        metric_report_manager: &Arc<Mutex<MetricReportManager>>,
    ) {
        // https://github.com/statsd/statsd/blob/master/docs/server.md
        // From statsd spec:
        // Multiple metrics can be received in a single packet if separated by the \n character.
        let metric_readings = message
            .trim()
            .lines()
            .map(KeyedMetricReading::from_statsd_str)
            // Drop strings that couldn't be parsed as a KeyedMetricReading
            .filter_map(|res| {
                if let Err(e) = &res {
                    warn!("{}", e)
                };
                res.ok()
            });

        for metric_reading in metric_readings {
            if let Err(e) = metric_report_manager
                .lock()
                .expect("Mutex poisoned!")
                .add_metric(metric_reading)
            {
                warn!("Error adding metric sent to StatsD server: {}", e);
            }
        }
    }
}

impl Default for StatsDServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use insta::assert_json_snapshot;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("test_counter:1|c", "test_gauge:2.0|g", "test_simple")]
    #[case("test_counter:1|c", "test_counter:1|c", "test_counter_aggregation")]
    #[case(
        "test_counter:1|c\ntest_gauge:2.0|g",
        "test_counter:1|c\ntest_gauge:10.0|g",
        "test_counter_and_gauge_aggregation"
    )]
    #[case(
        "test_histo:100|h\ntest_another_histo:20.0|h",
        "test_one_more_histo:35|h\ntest_another_histo:1000.0|h",
        "test_histogram_aggregation"
    )]
    fn test_process_statsd_message(
        #[case] statsd_message_a: &str,
        #[case] statsd_message_b: &str,
        #[case] test_name: &str,
    ) {
        let metric_report_manager = Arc::new(Mutex::new(MetricReportManager::new()));

        // Process first StatsD test message
        StatsDServer::process_statsd_message(statsd_message_a, &metric_report_manager);

        // Process second StatsD test message
        StatsDServer::process_statsd_message(statsd_message_b, &metric_report_manager);

        // Verify resulting metric report
        let metrics: BTreeMap<_, _> = metric_report_manager
            .lock()
            .unwrap()
            .take_heartbeat_metrics()
            .into_iter()
            .collect();

        assert_json_snapshot!(test_name, metrics);
    }
}
