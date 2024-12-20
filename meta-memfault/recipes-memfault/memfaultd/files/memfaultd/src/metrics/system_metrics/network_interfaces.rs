//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect Network Interface metric readings from /proc/net/dev
//! and /proc/net/wireless
//!
//! Example /proc/net/dev output:
//! Inter-|   Receive                                                |  Transmit
//!  face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
//!    lo:    2707      25    0    0    0     0          0         0     2707      25    0    0    0     0       0          0
//!  eth0:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0
//! wlan0: 10919408    8592    0    0    0     0          0         0   543095    4066    0    0    0     0       0          0
//!
//! Kernel docs:
//! https://docs.kernel.org/filesystems/proc.html#networking-info-in-proc-net
//! Extra information about /proc/net/wireless
//! https://hewlettpackard.github.io/wireless-tools/Linux.Wireless.Extensions.html
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::iter::zip;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::Utc;
use log::debug;
use nom::bytes::complete::tag;
use nom::character::complete::{alphanumeric1, i64, multispace0, multispace1, u64};
use nom::{
    combinator::opt,
    multi::count,
    sequence::{pair, preceded, terminated},
    IResult,
};

use crate::metrics::core_metrics::{
    METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_PREFIX,
    METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_SUFFIX,
    METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_PREFIX,
    METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_SUFFIX,
};
use crate::{
    metrics::{
        system_metrics::SystemMetricFamilyCollector, KeyedMetricReading, MetricReading,
        MetricStringKey,
    },
    util::time_measure::TimeMeasure,
};

use eyre::{eyre, ErrReport, Result};

const PROC_NET_DEV_PATH: &str = "/proc/net/dev";
const PROC_NET_WIRELESS_PATH: &str = "/proc/net/wireless";
pub const NETWORK_INTERFACE_METRIC_NAMESPACE: &str = "interface";
pub const METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX: &str = "bytes_per_second/rx";
pub const METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX: &str = "bytes_per_second/tx";

// Metric keys that are currently captured and reported
// by memfaultd.
// There is a lot of information in /proc/net/dev
// and the intention with this list is to use it
// to filter out the values read from it so that
// only the high-signal and widely-applicable metrics remain.
const NETWORK_INTERFACE_METRIC_KEYS: &[&str; 8] = &[
    METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX,
    "packets_per_second/rx",
    "errors_per_second/rx",
    "dropped_per_second/rx",
    METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX,
    "packets_per_second/tx",
    "errors_per_second/tx",
    "dropped_per_second/tx",
];

pub enum NetworkInterfaceMetricsConfig {
    Auto,
    Interfaces(HashSet<String>),
}

pub struct NetworkInterfaceMetricCollector<T: TimeMeasure> {
    config: NetworkInterfaceMetricsConfig,
    previous_readings_by_interface: HashMap<String, ProcNetDevReading<T>>,
}

#[derive(Clone)]
pub struct ProcNetDevReading<T: TimeMeasure> {
    stats: Vec<u64>,
    reading_time: T,
}

impl<T> NetworkInterfaceMetricCollector<T>
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync + 'static,
{
    pub fn new(config: NetworkInterfaceMetricsConfig) -> Self {
        Self {
            config,
            previous_readings_by_interface: HashMap::new(),
        }
    }

    fn interface_is_monitored(&self, interface: &str) -> bool {
        match &self.config {
            // Ignore loopback, tunnel, and dummy interfaces in Auto mode
            NetworkInterfaceMetricsConfig::Auto => {
                !(interface.starts_with("lo")
                    || interface.starts_with("tun")
                    || interface.starts_with("dummy"))
            }
            NetworkInterfaceMetricsConfig::Interfaces(configured_interfaces) => {
                configured_interfaces.contains(interface)
            }
        }
    }

    pub fn get_wireless_interface_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let path = Path::new(PROC_NET_WIRELESS_PATH);
        if !path.exists() {
            // Return early if wireless extension is not enabled
            return Ok(vec![]);
        }
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut wireless_metric_readings = vec![];
        for line in reader.lines() {
            if let Ok((interface_id, net_stats)) = Self::parse_proc_net_wireless_line(line?.trim())
            {
                // Ignore unmonitored interfaces
                if self.interface_is_monitored(&interface_id) {
                    let level = net_stats
                        .get(1)
                        .ok_or(eyre!("Missing level for {}", interface_id))?;
                    let rssi_key = MetricStringKey::from_str(
                        format!("interfaces/{}/rssi", interface_id).as_str(),
                    )
                    .map_err(|e| {
                        eyre!("Couldn't build link metric key for {}: {}", interface_id, e)
                    })?;

                    wireless_metric_readings
                        .extend([KeyedMetricReading::new_histogram(rssi_key, *level as f64)]);
                }
            } else {
                debug!("Couldn't parse /proc/net/wireless line");
            }
        }

        Ok(wireless_metric_readings)
    }

    pub fn get_network_interface_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        // Track if any lines in /proc/net/dev are parse-able
        // so we can alert user if none are
        let mut no_parseable_lines = true;

        let path = Path::new(PROC_NET_DEV_PATH);

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut net_metric_readings = vec![];
        let mut total_bytes_rx = 0;
        let mut total_bytes_tx = 0;

        for line in reader.lines() {
            // Discard errors - the assumption here is that we are only parsing
            // lines that follow the specified format and expect other lines in the file to error
            if let Ok((interface_id, net_stats)) = Self::parse_proc_net_dev_line(line?.trim()) {
                no_parseable_lines = false;

                // Ignore unmonitored interfaces
                if self.interface_is_monitored(&interface_id) {
                    if let Ok(mut readings) = self.calculate_network_metrics(
                        interface_id.to_string(),
                        ProcNetDevReading {
                            stats: net_stats,
                            reading_time: T::now(),
                        },
                        &mut total_bytes_rx,
                        &mut total_bytes_tx,
                    ) {
                        net_metric_readings.append(&mut readings);
                    }
                }
            }
        }

        // Check if we were able to parse at least one CPU metric reading
        if no_parseable_lines {
            Err(eyre!(
                    "No network metrics were collected from {} - is it a properly formatted /proc/net/dev file?",
                    PROC_NET_DEV_PATH
            ))
        } else {
            Ok(net_metric_readings)
        }
    }

    /// Parse a network interface name from a line of /proc/net/dev
    /// The network interface may be preceded by whitespace and will
    /// always be terminated with a ':'
    /// in a line that is followed by 16 number values
    fn parse_net_if(input: &str) -> IResult<&str, &str> {
        terminated(preceded(multispace0, alphanumeric1), tag(":"))(input)
    }

    /// Parse the CPU stats from the suffix of a /proc/net/dev line following the interface ID
    ///
    /// The first 8 values track RX traffic on the interface. The latter 8 track TX traffic.
    fn parse_interface_stats(input: &str) -> IResult<&str, Vec<u64>> {
        count(preceded(multispace1, u64), 16)(input)
    }

    /// Parse the output of a line of /proc/net/dev, returning
    /// a pair of the network interface that the parsed line corresponds
    /// to and the first 7 floats listed for it
    ///
    /// The first 8 values track RX traffic on the interface since boot with
    /// following names (in order):
    /// "bytes", "packets", "errs", "drop", "fifo", "frame", "compressed" "multicast"
    /// The latter 8 track TX traffic, with the following names:
    /// "bytes", "packets", "errs", "drop", "fifo", "colls", "carrier", "compressed"
    ///
    /// Important!!: The rest of this module assumes this is the ordering of values
    /// in the /proc/net/dev file
    fn parse_proc_net_dev_line(line: &str) -> Result<(String, Vec<u64>)> {
        let (_remaining, (interface_id, net_stats)) =
            pair(Self::parse_net_if, Self::parse_interface_stats)(line)
                .map_err(|e| eyre!("Failed to parse /proc/net/dev line: {}", e))?;
        Ok((interface_id.to_string(), net_stats))
    }

    fn parse_wireless_status(input: &str) -> IResult<&str, u64> {
        preceded(multispace1, u64)(input)
    }

    fn parse_wireless_stats(input: &str) -> IResult<&str, Vec<i64>> {
        count(terminated(preceded(multispace1, i64), opt(tag("."))), 2)(input)
    }

    /// Parse the output of a line of /proc/net/wireless, returning
    /// a pair of the network interface that the parsed line corresponds
    /// to and the 2 signed integer values for "link" and "level"
    fn parse_proc_net_wireless_line(line: &str) -> Result<(String, Vec<i64>)> {
        let (_remaining, (interface_id, (_status, wireless_stats))) = pair(
            Self::parse_net_if,
            pair(Self::parse_wireless_status, Self::parse_wireless_stats),
        )(line)
        .map_err(|e| eyre!("Failed to parse /proc/net/wireless line: {}", e))?;
        Ok((interface_id.to_string(), wireless_stats))
    }

    /// We need to account for potential rollovers in the
    /// /proc/net/dev counters, handled by this function
    fn counter_delta_with_overflow(current: u64, previous: u64) -> u64 {
        // The only time a counter's value would be less
        // that its previous value is if it rolled over
        // due to overflow - drop these readings that overlap
        // with an overflow
        if current < previous {
            // Need to detect if the counter rolled over at u32::MAX or u64::MAX
            current
                + ((if previous > u32::MAX as u64 {
                    u64::MAX
                } else {
                    u32::MAX as u64
                }) - previous)
        } else {
            current - previous
        }
    }

    /// Calculates network metrics     
    fn calculate_network_metrics(
        &mut self,
        interface: String,
        current_reading: ProcNetDevReading<T>,
        total_bytes_rx: &mut u64,
        total_bytes_tx: &mut u64,
    ) -> Result<Vec<KeyedMetricReading>> {
        // Check to make sure there was a previous reading to calculate a delta with
        if let Some(ProcNetDevReading {
            stats: previous_net_stats,
            reading_time: previous_reading_time,
        }) = self
            .previous_readings_by_interface
            .insert(interface.clone(), current_reading.clone())
        {
            // Bytes received is the first numeric value in a /proc/net/dev line
            let curr_interface_bytes_rx = current_reading
                .stats
                .first()
                .ok_or(eyre!("Current reading is missing bytes received value"))?;
            let prev_interface_bytes_rx = previous_net_stats
                .first()
                .ok_or(eyre!("Previous reading is missing bytes received value"))?;
            let interface_bytes_rx = Self::counter_delta_with_overflow(
                *curr_interface_bytes_rx,
                *prev_interface_bytes_rx,
            );

            // Bytes sent is the 9th numeric value in a /proc/net/dev line
            let curr_interface_bytes_tx = current_reading
                .stats
                .get(8)
                .ok_or(eyre!("Current reading is missing bytes sent value"))?;
            let prev_interface_bytes_tx = previous_net_stats
                .get(8)
                .ok_or(eyre!("Previous reading is missing bytes sent value"))?;
            let interface_bytes_tx = Self::counter_delta_with_overflow(
                *curr_interface_bytes_tx,
                *prev_interface_bytes_tx,
            );

            *total_bytes_rx += interface_bytes_rx;
            *total_bytes_tx += interface_bytes_tx;

            let interface_rx_key = MetricStringKey::from_str(
                format!(
                    "{}{}{}",
                    METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_PREFIX,
                    interface,
                    METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_SUFFIX
                )
                .as_str(),
            )
            .map_err(|e| eyre!("Couldn't construct metric key: {}", e))?;
            let interface_tx_key = MetricStringKey::from_str(
                format!(
                    "{}{}{}",
                    METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_PREFIX,
                    interface,
                    METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_SUFFIX
                )
                .as_str(),
            )
            .map_err(|e| eyre!("Couldn't construct metric key: {}", e))?;

            let _interface_core_metrics = [
                KeyedMetricReading::new_counter(interface_rx_key, interface_bytes_rx as f64),
                KeyedMetricReading::new_counter(interface_tx_key, interface_bytes_tx as f64),
            ];

            let current_period_rates =
                current_reading
                    .stats
                    .iter()
                    .zip(previous_net_stats)
                    .map(|(current, previous)| {
                        Self::counter_delta_with_overflow(*current, previous) as f64
                            / (current_reading
                                .reading_time
                                .since(&previous_reading_time)
                                .as_secs_f64())
                    });

            let net_keys_with_stats = zip(
                [
                    "bytes_per_second/rx",
                    "packets_per_second/rx",
                    "errors_per_second/rx",
                    "dropped_per_second/rx",
                    "fifo/rx",
                    "frame/rx",
                    "compressed/rx",
                    "multicast/rx",
                    "bytes_per_second/tx",
                    "packets_per_second/tx",
                    "errors_per_second/tx",
                    "dropped_per_second/tx",
                    "fifo/tx",
                    "colls/tx",
                    "carrier/tx",
                    "compressed/tx",
                ],
                current_period_rates,
            )
            // Filter out metrics we don't want memfaultd to include in reports like fifo and colls
            .filter(|(key, _)| NETWORK_INTERFACE_METRIC_KEYS.contains(key))
            .collect::<Vec<(&str, f64)>>();

            let timestamp = Utc::now();
            let readings = net_keys_with_stats
                .iter()
                .map(|(key, value)| -> Result<KeyedMetricReading, ErrReport> {
                    Ok(KeyedMetricReading::new(
                        MetricStringKey::from_str(&format!(
                            "{}/{}/{}",
                            NETWORK_INTERFACE_METRIC_NAMESPACE, interface, key
                        ))
                        .map_err(|e| eyre!(e))?,
                        MetricReading::Histogram {
                            value: *value,
                            timestamp,
                        },
                    ))
                })
                .collect::<Result<Vec<KeyedMetricReading>>>();
            Ok(readings?)
        } else {
            Ok(vec![])
        }
    }
}

impl<T> SystemMetricFamilyCollector for NetworkInterfaceMetricCollector<T>
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync + 'static,
{
    fn family_name(&self) -> &'static str {
        NETWORK_INTERFACE_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let mut network_metrics = self.get_network_interface_metrics()?;
        let net_wireless_metrics = self.get_wireless_interface_metrics()?;
        network_metrics.extend(net_wireless_metrics);
        Ok(network_metrics)
    }
}

#[cfg(test)]
mod test {

    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;

    use super::*;
    use crate::test_utils::TestInstant;

    #[rstest]
    #[case("   eth0:    2707      25    0    0    0     0          0         0     2707      25    0    0    0     0       0          0", "eth0")]
    #[case("wlan1:    2707      25    0    0    0     0          0         0     2707      25    0    0    0     0       0          0", "wlan1")]
    fn test_parse_netdev_line(#[case] proc_net_dev_line: &str, #[case] test_name: &str) {
        assert_json_snapshot!(test_name, 
                              NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(proc_net_dev_line).unwrap(),
                              {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
    }
    #[rstest]
    #[case("wlan0: 0000   56.  -54.  -256        0      0      0      0     38        0")]
    #[case("wlan0: 0000   56  -54  -256        0      0      0      0     38        0")]
    fn test_parse_netwireless_line(#[case] proc_net_dev_line: &str) {
        let result = NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_wireless_line(
            proc_net_dev_line,
        );
        assert!(result.is_ok());
        let (interface, stats) = result.unwrap();
        assert_eq!(interface, "wlan0");
        assert_eq!(stats, [56, -54]);
    }

    #[rstest]
    // Missing a colon after wlan0
    #[case("wlan0   2707      25    0    0    0     0          0         0     2707      25    0    0    0     0       0          0")]
    // Only 15 stat values instead of 16
    #[case("wlan0:   2707    0    0    0     0          0         0     2707      25    0    0    0     0       0          0")]
    fn test_fails_on_invalid_proc_net_dev_line(#[case] proc_net_dev_line: &str) {
        assert!(
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line
            )
            .is_err()
        )
    }

    #[rstest]
    #[case(
            "   eth0:    1000      25    0    0    0     0          0         0     2000      25    0    0    0     0       0          0",
            "   eth0:    2500      80    10   10   0     0          0         0     3000      50    0    0    0     0       0          0",
            "   eth0:    5000      100   15   15   0     0          0         0     5000      75    20   20   0     0       0          0",
            "basic_delta"
        )]
    #[case(
            "   eth0:    4294967293     25    0    0    0     0          0         0     2000      25    0    0    0     0       0          0",
            "   eth0:    2498           80    10   10   0     0          0         0     3000      50    0    0    0     0       0          0",
            "   eth0:    5000           100   15   15   0     0          0         0     5000      75    20   20   0     0       0          0",
            "with_overflow"
        )]
    fn test_net_if_metric_collector_calcs(
        #[case] proc_net_dev_line_a: &str,
        #[case] proc_net_dev_line_b: &str,
        #[case] proc_net_dev_line_c: &str,
        #[case] test_name: &str,
    ) {
        let mut net_metric_collector = NetworkInterfaceMetricCollector::<TestInstant>::new(
            NetworkInterfaceMetricsConfig::Interfaces(HashSet::from_iter(["eth0".to_string()])),
        );

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_a,
            )
            .unwrap();
        let reading_a = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_a =
            net_metric_collector.calculate_network_metrics(net_if, reading_a, &mut 0, &mut 0);
        assert!(result_a.unwrap().is_empty());

        TestInstant::sleep(Duration::from_secs(10));

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_b,
            )
            .unwrap();
        let reading_b = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_b =
            net_metric_collector.calculate_network_metrics(net_if, reading_b, &mut 0, &mut 0);

        assert!(result_b.is_ok());

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "a_b_metrics"),
                                  result_b.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });

        TestInstant::sleep(Duration::from_secs(30));

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_c,
            )
            .unwrap();
        let reading_c = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_c =
            net_metric_collector.calculate_network_metrics(net_if, reading_c, &mut 0, &mut 0);

        assert!(result_c.is_ok());

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "b_c_metrics"),
                                  result_c.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }

    #[rstest]
    #[case(
            "   eth0:    1000      25    0    0    0     0          0         0     2000      25    0    0    0     0       0          0",
            "   eth1:    2500      80    10   10   0     0          0         0     3000      50    0    0    0     0       0          0",
            "   eth0:    5000      100   15   15   0     0          0         0     5000      75    20   20   0     0       0          0",
            "   eth1:    3700      100   10   10   0     0          0         0     3200      50    0    0    0     0       0          0",
            true,
            "different_interfaces"
        )]
    fn test_net_if_metric_collector_different_if(
        #[case] proc_net_dev_line_a: &str,
        #[case] proc_net_dev_line_b: &str,
        #[case] proc_net_dev_line_c: &str,
        #[case] proc_net_dev_line_d: &str,
        #[case] use_auto_config: bool,
        #[case] test_name: &str,
    ) {
        let mut total_bytes_rx: u64 = 0;
        let mut total_bytes_tx: u64 = 0;

        let mut net_metric_collector =
            NetworkInterfaceMetricCollector::<TestInstant>::new(if use_auto_config {
                NetworkInterfaceMetricsConfig::Auto
            } else {
                NetworkInterfaceMetricsConfig::Interfaces(HashSet::from_iter(["eth1".to_string()]))
            });

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_a,
            )
            .unwrap();
        let reading_a = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_a = net_metric_collector.calculate_network_metrics(
            net_if,
            reading_a,
            &mut total_bytes_rx,
            &mut total_bytes_tx,
        );
        assert!(result_a.unwrap().is_empty());

        TestInstant::sleep(Duration::from_secs(10));

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_b,
            )
            .unwrap();
        let reading_b = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_b = net_metric_collector.calculate_network_metrics(
            net_if,
            reading_b,
            &mut total_bytes_rx,
            &mut total_bytes_tx,
        );
        assert!(result_b.unwrap().is_empty());

        TestInstant::sleep(Duration::from_secs(30));

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_c,
            )
            .unwrap();
        let reading_c = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_c = net_metric_collector.calculate_network_metrics(
            net_if,
            reading_c,
            &mut total_bytes_rx,
            &mut total_bytes_tx,
        );

        assert!(result_c.is_ok());
        // 2 readings are required to calculate metrics (since they are rates),
        // so we should only get actual metrics after processing reading_c
        // (which is the second eth0 reading)
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "a_c_metrics"),
                                  result_c.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });

        TestInstant::sleep(Duration::from_secs(30));

        let (net_if, stats) =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_proc_net_dev_line(
                proc_net_dev_line_d,
            )
            .unwrap();
        let reading_d = ProcNetDevReading {
            stats,
            reading_time: TestInstant::now(),
        };
        let result_d = net_metric_collector.calculate_network_metrics(
            net_if,
            reading_d,
            &mut total_bytes_rx,
            &mut total_bytes_tx,
        );

        assert!(result_d.is_ok());

        assert_eq!(total_bytes_tx, 3200);
        assert_eq!(total_bytes_rx, 5200);

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "b_d_metrics"),
                                  result_d.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }
    #[rstest]
    #[case(vec!["eth0".to_string(), "wlan1".to_string()], "eth1", false)]
    #[case(vec!["eth0".to_string(), "wlan1".to_string()], "eth0", true)]
    #[case(vec!["eth0".to_string(), "wlan1".to_string()], "enp0s10", false)]
    #[case(vec!["eth0".to_string(), "wlan1".to_string()], "wlan1", true)]
    fn test_interface_is_monitored(
        #[case] monitored_interfaces: Vec<String>,
        #[case] interface: &str,
        #[case] should_be_monitored: bool,
    ) {
        let net_metric_collector = NetworkInterfaceMetricCollector::<TestInstant>::new(
            NetworkInterfaceMetricsConfig::Interfaces(HashSet::from_iter(monitored_interfaces)),
        );
        assert_eq!(
            net_metric_collector.interface_is_monitored(interface),
            should_be_monitored
        )
    }
    #[rstest]
    #[case("eth1", true)]
    #[case("eth0", true)]
    #[case("enp0s10", true)]
    #[case("wlan1", true)]
    #[case("tun0", false)]
    #[case("dummy1", false)]
    #[case("lo1", false)]
    fn test_interface_is_monitored_auto(
        #[case] interface: &str,
        #[case] should_be_monitored: bool,
    ) {
        let net_metric_collector = NetworkInterfaceMetricCollector::<TestInstant>::new(
            NetworkInterfaceMetricsConfig::Auto,
        );
        assert_eq!(
            net_metric_collector.interface_is_monitored(interface),
            should_be_monitored
        )
    }
}
