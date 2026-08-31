//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::net::{SocketAddr, UdpSocket};

use eyre::{eyre, Result};
use log::{debug, warn};

use crate::metrics::KeyedMetricReading;

use super::MetricReading;
use super::MetricsMBox;

pub struct StatsDServer {
    legacy_gauge_aggregation: bool,
    legacy_key_names: bool,
    metrics_mailbox: MetricsMBox,
    listening_address: SocketAddr,
    socket: Option<UdpSocket>,
}

impl StatsDServer {
    pub fn new(
        legacy_gauge_aggregation: bool,
        legacy_key_names: bool,
        metrics_mailbox: MetricsMBox,
        listening_address: SocketAddr,
    ) -> StatsDServer {
        StatsDServer {
            legacy_gauge_aggregation,
            legacy_key_names,
            metrics_mailbox,
            listening_address,
            socket: None,
        }
    }

    fn init_socket(&mut self) -> Result<(), String> {
        let socket = match UdpSocket::bind(self.listening_address) {
            Ok(socket) => socket,
            Err(e) => {
                // Failing to bind is not fatal to memfaultd - but want to propagate warning
                warn!(
                    "Couldn't start StatsD server on {}: {} - StatsD metrics will not be collected",
                    self.listening_address, e
                );
                return Err(format!("could not bind the StatsD socket: {}", e));
            }
        };
        debug!(
            "StatsD listening on {}",
            socket.local_addr().unwrap_or(self.listening_address)
        );
        self.socket = Some(socket);
        Ok(())
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.init_socket()?;
        loop {
            if let Err(e) = self.run_once() {
                warn!("{}", e);
            }
        }
    }

    pub fn run_once(&self) -> Result<()> {
        let socket = self
            .socket
            .as_ref()
            .ok_or_else(|| eyre!("StatsD socket is not bound"))?;

        // This means that packets with > 1432 bytes are NOT supported
        // Clients must enforce a maximum message size of 1432 bytes or less
        let mut buf = [0; 1432];
        match socket.recv(&mut buf) {
            Ok(amt) => {
                let message = String::from_utf8_lossy(&buf[..amt]);
                self.process_statsd_message(&message);
                Ok(())
            }
            Err(e) => Err(eyre!("StatsD server socket error: {}", e)),
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
    use std::net::{IpAddr, Ipv4Addr};

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

    #[rstest]
    fn test_run_once_receives_datagram_and_forwards_readings(mut fixture: Fixture) {
        fixture
            .server
            .init_socket()
            .expect("failed to bind StatsD socket");

        // The fixture binds port 0, so read back the port the OS picked.
        let server_address = fixture
            .server
            .socket
            .as_ref()
            .expect("init() should have bound the socket")
            .local_addr()
            .expect("bound socket should have a local address");

        let client = UdpSocket::bind(EPHEMERAL_ADDRESS).expect("failed to bind client socket");
        client
            .send_to(b"test_counter:1|c\ntest_gauge:2.0|g", server_address)
            .expect("failed to send datagram");

        // The datagram is already queued in the socket's receive buffer, so a
        // single run_once() consumes it without racing the send above.
        fixture.server.run_once().expect("run_once() failed");

        let names = fixture
            .mock
            .take_messages()
            .into_iter()
            .flatten()
            .map(|reading| reading.name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["test_counter", "test_gauge"]);
    }

    #[rstest]
    fn test_init_fails_when_address_is_already_bound(mut fixture: Fixture) {
        let squatter = UdpSocket::bind(EPHEMERAL_ADDRESS).expect("failed to bind squatter socket");
        let taken_address = squatter
            .local_addr()
            .expect("bound socket should have a local address");

        // Point the server at the port the squatter is holding. Port 0 can
        // never collide, so the address has to be overridden here.
        fixture.server.listening_address = taken_address;

        let error = fixture
            .server
            .init_socket()
            .expect_err("init_socket() should fail when the address is already bound");
        assert!(
            error.contains("could not bind the StatsD socket"),
            "unexpected error: {}",
            error
        );
    }

    /// Port 0 asks the OS for an unused port, so tests never collide with eachother
    const EPHEMERAL_ADDRESS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    struct Fixture {
        server: StatsDServer,
        mock: ServiceMock<Vec<KeyedMetricReading>>,
    }

    #[fixture]
    fn fixture() -> Fixture {
        let mock = ServiceMock::new();
        let server = StatsDServer::new(false, false, mock.mbox.clone(), EPHEMERAL_ADDRESS);

        Fixture { server, mock }
    }

    #[fixture]
    fn fixture_legacy_gauge_aggregation() -> Fixture {
        let mock = ServiceMock::new();
        let server = StatsDServer::new(true, false, mock.mbox.clone(), EPHEMERAL_ADDRESS);

        Fixture { server, mock }
    }
}
