//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect Network Interface metric readings from /proc/net/dev
//! /proc/net/wireless, and /proc/net/sockstat
//!
//! Example /proc/net/dev output:
//! Inter-|   Receive                                                |  Transmit
//!  face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
//!    lo:    2707      25    0    0    0     0          0         0     2707      25    0    0    0     0       0          0
//!  eth0:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0
//! wlan0: 10919408    8592    0    0    0     0          0         0   543095    4066    0    0    0     0       0          0
//!
//! Example /proc/net/sockstat output:
//! sockets: used 97
//! TCP: inuse 7 orphan 0 tw 0 alloc 8 mem 0
//! UDP: inuse 4 mem 2
//! UDPLITE: inuse 0
//! RAW: inuse 0
//! FRAG: inuse 0 memory 0
//!
//! Kernel docs:
//! https://docs.kernel.org/filesystems/proc.html#networking-info-in-proc-net
//! Extra information about /proc/net/wireless
//! https://hewlettpackard.github.io/wireless-tools/Linux.Wireless.Extensions.html
use std::collections::{HashMap, HashSet};
use std::fs::{read_to_string, File};
use std::io::{BufRead, BufReader};
use std::iter::zip;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::Utc;
use itertools::Itertools;
use log::debug;
use nom::bytes::complete::tag;
use nom::character::complete::{alphanumeric1, i64, multispace0, multispace1, space1, u64};
use nom::sequence::delimited;
use nom::Parser;
use nom::{
    combinator::opt,
    multi::count,
    sequence::{pair, preceded, terminated},
    IResult,
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
const PROC_NET_SOCKSTAT_PATH: &str = "/proc/net/sockstat";
pub const NETWORK_INTERFACE_METRIC_NAMESPACE: &str = "interface";
pub const METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX: &str = "bytes_per_second/rx";
pub const METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX: &str = "bytes_per_second/tx";
pub const METRIC_INTERFACE_NET_SOCKETS_PREFIX: &str = "net/sockets";

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

// NOTE: socket count doesn't need any timing data, similar to FD metrics
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

    fn collect(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let network_metrics = self
            .get_network_interface_metrics()
            .inspect_err(|e| {
                debug!(
                    "unable to collect metrics from {}: {}",
                    PROC_NET_DEV_PATH, e
                )
            })
            .ok();
        let net_wireless_metrics = self
            .get_wireless_interface_metrics()
            .inspect_err(|e| {
                debug!(
                    "unable to collect metrics from {}: {}",
                    PROC_NET_WIRELESS_PATH, e
                )
            })
            .ok();
        let socket_count_metrics = self
            .get_socket_count_metrics()
            .inspect_err(|e| {
                debug!(
                    "unable to collect metrics from {}: {}",
                    PROC_NET_SOCKSTAT_PATH, e
                )
            })
            .ok();

        let res = [network_metrics, net_wireless_metrics, socket_count_metrics]
            .into_iter()
            .while_some()
            .flatten()
            .collect::<Vec<KeyedMetricReading>>();

        if res.is_empty() {
            Err(eyre!(
                "unable to collect network interface metrics. See previously emitted warnings"
            ))
        } else {
            Ok(res)
        }
    }

    fn interface_is_monitored(&self, interface: &str) -> bool {
        match &self.config {
            // Ignore loopback, tunnel, veth, usb, and dummy interfaces in Auto mode
            NetworkInterfaceMetricsConfig::Auto => {
                !(interface.starts_with("lo")
                    || interface.starts_with("tun")
                    || interface.starts_with("dummy")
                    || interface.starts_with("veth")
                    || interface.starts_with("usb"))
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
        for line in reader.lines().map_while(Result::ok) {
            if let Ok((interface_id, net_stats)) = Self::parse_proc_net_wireless_line(line.trim()) {
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

    pub fn get_socket_count_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let (sockets_inuse, tcp_inuse, tcp_orphan, tcp_tw, udp_inuse) = {
            let path = Path::new(PROC_NET_SOCKSTAT_PATH);
            let sockstat_contents = read_to_string(path)?;
            let sockstat_data = Self::parse_sockstat(sockstat_contents)?;
            (
                sockstat_data[0],
                sockstat_data[1],
                sockstat_data[2],
                sockstat_data[3],
                sockstat_data[4],
            )
        };

        Ok(vec![
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("net/sockets/tcp_inuse"),
                tcp_inuse as f64,
            ),
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("net/sockets/sockets_inuse"),
                sockets_inuse as f64,
            ),
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("net/sockets/tcp_tw"),
                tcp_tw as f64,
            ),
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("net/sockets/tcp_orphan"),
                tcp_orphan as f64,
            ),
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("net/sockets/udp_inuse"),
                udp_inuse as f64,
            ),
        ])
    }

    pub fn get_network_interface_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        // Track if any lines in /proc/net/dev are parse-able
        // so we can alert user if none are
        let mut no_parseable_lines = true;

        let path = Path::new(PROC_NET_DEV_PATH);

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut net_metric_readings = vec![];

        for line in reader.lines().map_while(Result::ok) {
            // Discard errors - the assumption here is that we are only parsing
            // lines that follow the specified format and expect other lines in the file to error
            if let Ok((interface_id, net_stats)) = Self::parse_proc_net_dev_line(line.trim()) {
                no_parseable_lines = false;

                // Ignore unmonitored interfaces
                if self.interface_is_monitored(&interface_id) {
                    if let Ok(mut readings) = self.calculate_network_metrics(
                        interface_id.to_string(),
                        ProcNetDevReading {
                            stats: net_stats,
                            reading_time: T::now(),
                        },
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

    /// Parses the contents of /proc/net/sockstat.
    /// Example:
    /// ```console
    /// sockets: used 97
    /// TCP: inuse 7 orphan 0 tw 0 alloc 8 mem 0
    /// UDP: inuse 4 mem 2
    /// UDPLITE: inuse 0
    /// RAW: inuse 0
    /// FRAG: inuse 0 memory 0
    /// ```
    /// Currently should only ever resolve in a vec of five u64s,
    /// those being `sockets_inuse`, `tcp_inuse`, `tcp_orphan`, `tcp_tw`, and `udp_inuse`.
    fn parse_sockstat(input: String) -> Result<Vec<u64>> {
        let input = input.as_str();
        let (remainder, sockets_inuse) = Self::parse_sockstat_sockets_line(input)
            .map_err(|e| eyre!("failed parsing line describing socket usage: {}", e))?;

        let (remainder, mut tcp_line_stats) = Self::parse_sockstat_line(remainder, 5)
            .map_err(|e| eyre!("failed parsing line regarding TCP socket stats: {}", e))?;
        // only care about inuse, orphan, tw
        tcp_line_stats = tcp_line_stats[0..3].to_vec();

        // this is probably overkill to be using this to capture 1 number from this line
        // but I'd rather avoid code duplication atm
        let (_remainder, mut udp_line_stats) = Self::parse_sockstat_line(remainder, 2)
            .map_err(|e| eyre!("failed parsing line regarding UDP socket stats: {}", e))?;
        udp_line_stats = udp_line_stats[0..1].to_vec();

        let mut res = vec![sockets_inuse];
        res.append(&mut tcp_line_stats);
        res.append(&mut udp_line_stats);

        Ok(res)
    }
    fn parse_sockstat_line(input: &str, num_entries: usize) -> IResult<&str, Vec<u64>> {
        terminated(
            preceded(
                Self::parse_tcp_or_udp,
                count(preceded(Self::parse_sockstat_word, u64), num_entries),
            ),
            multispace1,
        )(input)
    }
    fn parse_tcp_or_udp(input: &str) -> IResult<&str, &str> {
        tag("TCP:").or(tag("UDP:")).parse(input)
    }
    fn parse_sockstat_word(input: &str) -> IResult<&str, &str> {
        delimited(space1, tag("inuse"), space1)
            .or(delimited(space1, tag("orphan"), space1))
            .or(delimited(space1, tag("tw"), space1))
            .or(delimited(space1, tag("alloc"), space1))
            .or(delimited(space1, tag("mem"), space1))
            .parse(input)
    }
    fn parse_sockstat_sockets_line(input: &str) -> IResult<&str, u64> {
        terminated(preceded(tag("sockets: used "), u64), multispace1)(input)
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

    /// Parse the wireless status from a line of /proc/net/wireless
    fn parse_wireless_status(input: &str) -> IResult<&str, u64> {
        preceded(multispace1, u64)(input)
    }

    /// Parse the wireless stats from a line of `/proc/net/wireless`.
    ///
    /// Example contents of the `/proc/net/wireless` file:
    ///
    /// ```plaintext
    /// Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
    ///  face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
    /// wlp0s20f3: 0000   64.  -46.  -256        0      0      0      0    255        0
    /// ```
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

    /// Calculates network metrics
    fn calculate_network_metrics(
        &mut self,
        interface: String,
        current_reading: ProcNetDevReading<T>,
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
            let interface_bytes_rx = curr_interface_bytes_rx.checked_sub(*prev_interface_bytes_rx);

            // Bytes sent is the 9th numeric value in a /proc/net/dev line
            let curr_interface_bytes_tx = current_reading
                .stats
                .get(8)
                .ok_or(eyre!("Current reading is missing bytes sent value"))?;
            let prev_interface_bytes_tx = previous_net_stats
                .get(8)
                .ok_or(eyre!("Previous reading is missing bytes sent value"))?;
            let interface_bytes_tx = curr_interface_bytes_tx.checked_sub(*prev_interface_bytes_tx);

            let interface_rx_key = MetricStringKey::from_str(
                format!("interface/{}/total_bytes/rx", interface).as_str(),
            )
            .map_err(|e| eyre!("Couldn't construct metric key: {}", e))?;

            let interface_tx_key = MetricStringKey::from_str(
                format!("interface/{}/total_bytes/tx", interface).as_str(),
            )
            .map_err(|e| eyre!("Couldn't construct metric key: {}", e))?;

            let mut interface_counter_readings = [
                (interface_tx_key, interface_bytes_tx),
                (interface_rx_key, interface_bytes_rx),
            ]
            .into_iter()
            .filter_map(|(key, value)| {
                value.map(|v| KeyedMetricReading::new_counter(key, v as f64))
            })
            .collect::<Vec<KeyedMetricReading>>();

            let current_period_rates =
                current_reading
                    .stats
                    .iter()
                    .zip(previous_net_stats)
                    .map(|(current, previous)| {
                        if *current >= previous {
                            Some(
                                (*current - previous) as f64
                                    / current_reading
                                        .reading_time
                                        .since(&previous_reading_time)
                                        .as_secs_f64(),
                            )
                        } else {
                            None
                        }
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
            .filter_map(|(key, value)| {
                match (NETWORK_INTERFACE_METRIC_KEYS.contains(&key), value) {
                    (true, Some(value)) => Some((key, value)),
                    _ => None,
                }
            })
            .collect::<Vec<(&str, f64)>>();

            let timestamp = Utc::now();
            let mut readings = net_keys_with_stats
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
                .collect::<Result<Vec<KeyedMetricReading>>>()?;

            readings.append(&mut interface_counter_readings);

            Ok(readings)
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
        self.collect()
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
        let result_a = net_metric_collector.calculate_network_metrics(net_if, reading_a);
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
        let result_b = net_metric_collector.calculate_network_metrics(net_if, reading_b);

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
        let result_c = net_metric_collector.calculate_network_metrics(net_if, reading_c);

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
        let result_a = net_metric_collector.calculate_network_metrics(net_if, reading_a);
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
        let result_b = net_metric_collector.calculate_network_metrics(net_if, reading_b);
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
        let result_c = net_metric_collector.calculate_network_metrics(net_if, reading_c);

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
        let result_d = net_metric_collector.calculate_network_metrics(net_if, reading_d);

        assert!(result_d.is_ok());

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
    #[case("vethcdd37e7", false)]
    #[case("usb1047", false)]
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

    #[rstest]
    #[case("sockets: used 97 ", Some(97), "")]
    #[case("sockets: used", None, "")]
    #[case(
        "sockets: used 97
        TCP: inuse 7 orphan 0 tw 0 alloc 8 mem 0",
        Some(97),
        "TCP: inuse 7 orphan 0 tw 0 alloc 8 mem 0"
    )]
    fn test_parse_sockstat_sockets_line(
        #[case] line: &str,
        #[case] expected: Option<u64>,
        #[case] expected_remainder: &str,
    ) {
        let res = NetworkInterfaceMetricCollector::<TestInstant>::parse_sockstat_sockets_line(line);
        match expected {
            None => assert!(res.is_err()),
            Some(expected_no) => match res {
                Ok((remainder, res_no)) => {
                    assert_eq!(res_no, expected_no);
                    assert_eq!(remainder, expected_remainder);
                }
                Err(e) => panic!(
                    "failed parsing sockstat sockets line when expected success: {}",
                    e
                ),
            },
        }
    }
    #[rstest]
    #[case("TCP: inuse 7 orphan 0 tw 0 alloc 8 mem 0 ", Some(vec![7, 0, 0, 8, 0]), "", 5)]
    #[case("UDP: inuse 4 mem 2 ", Some(vec![4, 2]), "", 2)]
    #[case("JEFF!: inuse 4 mem lol ", None, "", 2)]
    #[case("JEFF!: inuse 4 mem 12345 yomama 1 ", None, "", 3)]
    #[case("JEFF!: inuse 4 mem 12345 yomama ", None, "", 3)]
    fn test_parse_sockstat_line(
        #[case] line: &str,
        #[case] expected: Option<Vec<u64>>,
        #[case] expected_remainder: &str,
        #[case] num_entries: usize,
    ) {
        let res =
            NetworkInterfaceMetricCollector::<TestInstant>::parse_sockstat_line(line, num_entries);
        match expected {
            None => assert!(res.is_err()),
            Some(expected_no) => match res {
                Ok((remainder, res_no)) => {
                    assert_eq!(res_no, expected_no);
                    assert_eq!(remainder, expected_remainder);
                }
                Err(e) => panic!("failed parsing sockstat line when expected success: {}", e),
            },
        }
    }
    #[rstest]
    #[case(
        "sockets: used 97
        TCP: inuse 7 orphan 0 tw 0 alloc 8 mem 0
        UDP: inuse 4 mem 2
        UDPLITE: inuse 0
        RAW: inuse 0
        FRAG: inuse 0 memory 0
        ".into(),
        Some(vec![97, 7, 0, 0, 4])
    )]
    fn test_parse_sockstat(#[case] contents: String, #[case] expected: Option<Vec<u64>>) {
        let res = NetworkInterfaceMetricCollector::<TestInstant>::parse_sockstat(contents);
        match expected {
            None => assert!(res.is_err()),
            Some(expected_vec) => {
                let res_vec = res.expect("expected to be valid sockstat report");
                assert_eq!(expected_vec, res_vec);
            }
        }
    }
}
