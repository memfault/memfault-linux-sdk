//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    ops::Sub,
    sync::{Arc, Mutex},
    time::Duration,
};

use eyre::Result;

use crate::{
    config::{ConnectivityMonitorConfig, ConnectivityMonitorTarget},
    metrics::MetricReportManager,
    util::{can_connect::CanConnect, time_measure::TimeMeasure},
};

const METRIC_CONNECTED_TIME: &str = "connectivity_connected_time_ms";
const METRIC_EXPECTED_CONNECTED_TIME: &str = "connectivity_expected_time_ms";

pub struct ConnectivityMonitor<T, U> {
    targets: Vec<ConnectivityMonitorTarget>,
    interval: Duration,
    last_checked_at: Option<T>,
    heartbeat_manager: Arc<Mutex<MetricReportManager>>,
    connection_checker: U,
}

impl<T, U> ConnectivityMonitor<T, U>
where
    T: TimeMeasure + Copy + Ord + Sub<T, Output = Duration>,
    U: CanConnect,
{
    pub fn new(
        config: &ConnectivityMonitorConfig,
        heartbeat_manager: Arc<Mutex<MetricReportManager>>,
    ) -> Self {
        Self {
            targets: config.targets.clone(),
            interval: config.interval_seconds,
            last_checked_at: None,
            heartbeat_manager,
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

        let mut store = self.heartbeat_manager.lock().expect("Mutex Poisoned");
        store.add_to_counter(METRIC_CONNECTED_TIME, connected_duration.as_millis() as f64)?;
        store.add_to_counter(
            METRIC_EXPECTED_CONNECTED_TIME,
            since_last_reading.as_millis() as f64,
        )?;

        self.last_checked_at = Some(now);

        Ok(())
    }

    pub fn interval_seconds(&self) -> Duration {
        self.interval
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        net::IpAddr,
        str::FromStr,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use insta::assert_json_snapshot;
    use rstest::rstest;

    use super::ConnectivityMonitor;
    use crate::test_utils::{TestConnectionChecker, TestInstant};
    use crate::{
        config::{ConnectionCheckProtocol, ConnectivityMonitorConfig, ConnectivityMonitorTarget},
        metrics::MetricReportManager,
    };

    #[rstest]
    fn test_while_connected() {
        let heartbeat_manager = Arc::new(Mutex::new(MetricReportManager::new()));
        let config = ConnectivityMonitorConfig {
            targets: vec![ConnectivityMonitorTarget {
                host: IpAddr::from_str("8.8.8.8").unwrap(),
                port: 443,
                protocol: ConnectionCheckProtocol::Tcp,
            }],
            interval_seconds: Duration::from_secs(15),
            timeout_seconds: Duration::from_secs(10),
        };
        let mut connectivity_monitor =
            ConnectivityMonitor::<TestInstant, TestConnectionChecker>::new(
                &config,
                heartbeat_manager.clone(),
            );

        TestConnectionChecker::connect();

        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");

        TestInstant::sleep(Duration::from_secs(30));
        connectivity_monitor
            .update_connected_time()
            .expect("Couldn't update connected time monitor!");

        let metrics = heartbeat_manager.lock().unwrap().take_heartbeat_metrics();

        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();

        assert_json_snapshot!(sorted_metrics);
    }

    #[rstest]
    fn test_half_connected_half_disconnected() {
        let heartbeat_manager = Arc::new(Mutex::new(MetricReportManager::new()));
        let config = ConnectivityMonitorConfig {
            targets: vec![ConnectivityMonitorTarget {
                host: IpAddr::from_str("8.8.8.8").unwrap(),
                port: 443,
                protocol: ConnectionCheckProtocol::Tcp,
            }],
            interval_seconds: Duration::from_secs(15),
            timeout_seconds: Duration::from_secs(10),
        };
        let mut connectivity_monitor =
            ConnectivityMonitor::<TestInstant, TestConnectionChecker>::new(
                &config,
                heartbeat_manager.clone(),
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
        let metrics = heartbeat_manager.lock().unwrap().take_heartbeat_metrics();

        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();
        assert_json_snapshot!(sorted_metrics);
    }

    #[rstest]
    fn test_fully_disconnected() {
        let heartbeat_manager = Arc::new(Mutex::new(MetricReportManager::new()));
        let config = ConnectivityMonitorConfig {
            targets: vec![ConnectivityMonitorTarget {
                host: IpAddr::from_str("8.8.8.8").unwrap(),
                port: 443,
                protocol: ConnectionCheckProtocol::Tcp,
            }],
            interval_seconds: Duration::from_secs(15),
            timeout_seconds: Duration::from_secs(10),
        };
        let mut connectivity_monitor =
            ConnectivityMonitor::<TestInstant, TestConnectionChecker>::new(
                &config,
                heartbeat_manager.clone(),
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
        let metrics = heartbeat_manager.lock().unwrap().take_heartbeat_metrics();

        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();

        assert_json_snapshot!(sorted_metrics);
    }
}
