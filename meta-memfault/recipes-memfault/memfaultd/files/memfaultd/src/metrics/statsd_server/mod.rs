//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::net::SocketAddr;
use std::net::UdpSocket;

use eyre::Result;
use log::warn;

use crate::metrics::KeyedMetricReading;

use super::MetricReading;
use super::MetricsMBox;

pub struct StatsDServer {
    legacy_gauge_aggregation: bool,
    legacy_key_names: bool,
    metrics_mailbox: MetricsMBox,
}

impl StatsDServer {
    pub fn new(
        legacy_gauge_aggregation: bool,
        legacy_key_names: bool,
        metrics_mailbox: MetricsMBox,
    ) -> StatsDServer {
        StatsDServer {
            legacy_gauge_aggregation,
            legacy_key_names,
            metrics_mailbox,
        }
    }

    pub fn run(&self, listening_address: SocketAddr) -> Result<()> {
        let socket = UdpSocket::bind(listening_address)?;
        loop {
            // This means that packets with > 1432 bytes are NOT supported
            // Clients must enforce a maximum message size of 1432 bytes or less
            let mut buf = [0; 1432];
            match socket.recv(&mut buf) {
                Ok(amt) => {
                    let message = String::from_utf8_lossy(&buf[..amt]);
                    self.process_statsd_message(&message)
                }
                Err(e) => warn!("Statsd server socket error: {}", e),
            }
        }
    }

    fn process_statsd_message(&self, message: &str) {
        // https://github.com/statsd/statsd/blob/master/docs/server.md
        // From statsd spec:
        // Multiple metrics can be received in a single packet if separated by the \n character.
        let metric_readings = message
            .trim()
            .lines()
            .map(|line| KeyedMetricReading::from_statsd_str(line, self.legacy_key_names))
            // Drop strings that couldn't be parsed as a KeyedMetricReading
            .filter_map(|res| {
                if let Err(e) = &res {
                    warn!("{}", e)
                };

                if self.legacy_gauge_aggregation {
                    match res {
                        // If legacy_gauge_aggregation is enabled, convert
                        // Gauges to Histograms on ingestion
                        Ok(KeyedMetricReading {
                            name,
                            value: MetricReading::Gauge { value, timestamp },
                        }) => Some(KeyedMetricReading {
                            name,
                            value: MetricReading::Histogram { value, timestamp },
                        }),
                        // legacy_gauge_aggretation has no affect on non-Gauge
                        // readings
                        _ => res.ok(),
                    }
                } else {
                    res.ok()
                }
            })
            .collect();

        if let Err(e) = self.metrics_mailbox.send_and_forget(metric_readings) {
            warn!("Error adding metric sent to StatsD server: {}", e);
        }
    }
}

#[cfg(test)]
mod test {
    use crate::metrics::TakeMetrics;
    use insta::{assert_json_snapshot, with_settings};
    use rstest::{fixture, rstest};
    use ssf::ServiceMock;

    use super::*;

    #[rstest]
    #[case("test_counter:1|c", "test_gauge:2.0|g", "test_simple")]
    #[case("test-counter:1|c", "test-gauge:2.0|g", "test_simple_dashes")]
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
        mut fixture: Fixture,
    ) {
        // Process first StatsD test message
        fixture.server.process_statsd_message(statsd_message_a);

        // Process second StatsD test message
        fixture.server.process_statsd_message(statsd_message_b);
        with_settings!({sort_maps => true}, {
        assert_json_snapshot!(test_name, fixture.mock.take_metrics().unwrap());
        });
    }

    #[rstest]
    #[case("test-gauge:1|g", "test-gauge:2.0|g", "test_legacy_gauge_aggregation")]
    #[case(
        "test_counter:1|c\ntest_gauge:2.0|g",
        "test_counter:1|c\ntest_gauge:10.0|g",
        "test_counter_and_legacy_gauge_aggregation"
    )]
    fn test_process_statsd_message_with_legacy_gauge_aggregation(
        #[case] statsd_message_a: &str,
        #[case] statsd_message_b: &str,
        #[case] test_name: &str,
        mut fixture_legacy_gauge_aggregation: Fixture,
    ) {
        // Process first StatsD test message
        fixture_legacy_gauge_aggregation
            .server
            .process_statsd_message(statsd_message_a);

        // Process second StatsD test message
        fixture_legacy_gauge_aggregation
            .server
            .process_statsd_message(statsd_message_b);
        with_settings!({sort_maps => true}, {
        assert_json_snapshot!(test_name, fixture_legacy_gauge_aggregation.mock.take_metrics().unwrap());
        });
    }

    struct Fixture {
        server: StatsDServer,
        mock: ServiceMock<Vec<KeyedMetricReading>>,
    }

    #[fixture]
    fn fixture() -> Fixture {
        let mock = ServiceMock::new();
        let server = StatsDServer::new(false, false, mock.mbox.clone());

        Fixture { server, mock }
    }

    #[fixture]
    fn fixture_legacy_gauge_aggregation() -> Fixture {
        let mock = ServiceMock::new();
        let server = StatsDServer::new(true, false, mock.mbox.clone());

        Fixture { server, mock }
    }
}
