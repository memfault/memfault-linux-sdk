//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{ops::Sub, time::Duration};

use eyre::Result;

use crate::{
    config::{ConnectivityMonitorConfig, ConnectivityMonitorTarget},
    metrics::{
        core_metrics::{METRIC_CONNECTED_TIME, METRIC_EXPECTED_CONNECTED_TIME},
        KeyedMetricReading, MetricStringKey, MetricsMBox,
    },
    util::{can_connect::CanConnect, time_measure::TimeMeasure},
};

pub struct ConnectivityMonitor<T, U> {
    targets: Vec<ConnectivityMonitorTarget>,
    interval: Duration,
    last_checked_at: Option<T>,
    metrics_mbox: MetricsMBox,
    connection_checker: U,
}

impl<T, U> ConnectivityMonitor<T, U>
where
    T: TimeMeasure + Copy + Ord + Sub<T, Output = Duration>,
    U: CanConnect,
{
    pub fn new(config: &ConnectivityMonitorConfig, metrics_mbox: MetricsMBox) -> Self {
        Self {
            targets: config.targets.clone(),
            interval: config.interval_seconds,
            last_checked_at: None,
            metrics_mbox,
            connection_checker: U::new(config.timeout_seconds),
        }
    }

    fn is_connected(&self) -> bool {
        self.targets
            .iter()
            .any(|ConnectivityMonitorTarget { host, port, .. }| {
                self.connection_checker.can_connect(host, *port).is_ok()
            })
    }

    pub fn update_connected_time(&mut self) -> Result<()> {
        let now = T::now();
        let since_last_reading = self.last_checked_at.unwrap_or(now).elapsed();
        let connected_duration = if self.is_connected() {
            since_last_reading
        } else {
            Duration::ZERO
        };

        let metrics = vec![
            KeyedMetricReading::add_to_counter(
                MetricStringKey::from(METRIC_CONNECTED_TIME),
                connected_duration.as_millis() as f64,
            ),
            KeyedMetricReading::add_to_counter(
                MetricStringKey::from(METRIC_EXPECTED_CONNECTED_TIME),
                since_last_reading.as_millis() as f64,
            ),
        ];
        self.metrics_mbox.send_and_forget(metrics)?;

        self.last_checked_at = Some(now);

        Ok(())
    }

    pub fn interval_seconds(&self) -> Duration {
        self.interval
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, net::IpAddr, str::FromStr, time::Duration};

    use insta::assert_json_snapshot;
    use rstest::rstest;
    use ssf::ServiceMock;

    use super::ConnectivityMonitor;
    use crate::test_utils::{TestConnectionChecker, TestInstant};
    use crate::{
        config::{ConnectionCheckProtocol, ConnectivityMonitorConfig, ConnectivityMonitorTarget},
        metrics::TakeMetrics,
    };

    #[rstest]
    fn test_while_connected() {
        let config = ConnectivityMonitorConfig {
            targets: vec![ConnectivityMonitorTarget {
                host: IpAddr::from_str("8.8.8.8").unwrap(),
                port: 443,
                protocol: ConnectionCheckProtocol::Tcp,
            }],
            interval_seconds: Duration::from_secs(15),
            timeout_seconds: Duration::from_secs(10),
        };
        let mut metrics_mock = ServiceMock::new();
        let mut connectivity_monitor =
            ConnectivityMonitor::<TestInstant, TestConnectionChecker>::new(
                &config,
                metrics_mock.mbox.clone(),
            );

        TestConnectionChecker::connect();

        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");

        TestInstant::sleep(Duration::from_secs(30));
        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");

        let metrics = metrics_mock.take_metrics().unwrap();

        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();

        assert_json_snapshot!(sorted_metrics);
    }

    #[rstest]
    fn test_half_connected_half_disconnected() {
        let config = ConnectivityMonitorConfig {
            targets: vec![ConnectivityMonitorTarget {
                host: IpAddr::from_str("8.8.8.8").unwrap(),
                port: 443,
                protocol: ConnectionCheckProtocol::Tcp,
            }],
            interval_seconds: Duration::from_secs(15),
            timeout_seconds: Duration::from_secs(10),
        };
        let mut metrics_mock = ServiceMock::new();
        let mut connectivity_monitor =
            ConnectivityMonitor::<TestInstant, TestConnectionChecker>::new(
                &config,
                metrics_mock.mbox.clone(),
            );

        TestConnectionChecker::connect();

        // Initial reading
        connectivity_monitor.update_connected_time().unwrap();

        TestInstant::sleep(Duration::from_secs(30));
        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");

        TestConnectionChecker::disconnect();

        TestInstant::sleep(Duration::from_secs(30));
        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");
        let metrics = metrics_mock.take_metrics().unwrap();

        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();
        assert_json_snapshot!(sorted_metrics);
    }

    #[rstest]
    fn test_fully_disconnected() {
        let config = ConnectivityMonitorConfig {
            targets: vec![ConnectivityMonitorTarget {
                host: IpAddr::from_str("8.8.8.8").unwrap(),
                port: 443,
                protocol: ConnectionCheckProtocol::Tcp,
            }],
            interval_seconds: Duration::from_secs(15),
            timeout_seconds: Duration::from_secs(10),
        };
        let mut metrics_mock = ServiceMock::new();
        let mut connectivity_monitor =
            ConnectivityMonitor::<TestInstant, TestConnectionChecker>::new(
                &config,
                metrics_mock.mbox.clone(),
            );

        TestConnectionChecker::disconnect();

        // Initial reading
        connectivity_monitor.update_connected_time().unwrap();

        TestInstant::sleep(Duration::from_secs(30));
        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");
        TestInstant::sleep(Duration::from_secs(30));
        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");
        let metrics = metrics_mock.take_metrics().unwrap();

        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();

        assert_json_snapshot!(sorted_metrics);
    }
}
