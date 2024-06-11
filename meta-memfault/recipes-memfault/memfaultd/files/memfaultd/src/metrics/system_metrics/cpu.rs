//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect CPU metric readings from /proc/stat
//!
//! This module parses CPU statistics from /proc/stat and
//! constructs KeyedMetricReadings based on those statistics.
//! Because the /proc/stat values are accumulations since boot,
//! a "previous reading" (stored in CpuMetricCollector) is
//! required to calculate the time each CPU core
//! has spent in each state since the last reading.
//!
//! Example /proc/stat contents:
//! cpu  326218 0 178980 36612114 6054 0 11961 0 0 0
//! cpu0 77186 0 73689 9126238 1353 0 6352 0 0 0
//! cpu1 83902 0 35260 9161039 1524 0 1865 0 0 0
//! cpu2 83599 0 35323 9161010 1676 0 1875 0 0 0
//! cpu3 81530 0 34707 9163825 1500 0 1867 0 0 0
//! intr 95400676 0 9795 1436573 0 0 0 0 0 0 0 0 93204555 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 77883 0 530 0 0 1523 0 0 468762 0 0 97412 103573 0 70 0 0 0 0 0
//! ctxt 9591503
//! btime 1714309294
//! processes 9416
//! procs_running 1
//! procs_blocked 0
//! softirq 47765068 15 3173702 0 541726 82192 0 1979 41497887 0 2467567
//!
//! Only the lines that start with "cpu" are currently
//! processed into metric readings by this module - the rest are discarded.
//!
//! See additional Linux kernel documentation on /proc/stat here:
//! https://docs.kernel.org/filesystems/proc.html#miscellaneous-kernel-statistics-in-proc-stat
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::iter::zip;
use std::ops::Add;
use std::path::Path;
use std::str::FromStr;

use chrono::Utc;
use nom::{
    bytes::complete::tag,
    character::complete::{digit0, space1},
    multi::count,
    number::complete::double,
    sequence::{pair, preceded},
    IResult,
};

use crate::metrics::{KeyedMetricReading, MetricReading, MetricStringKey};
use eyre::{eyre, ErrReport, Result};

const PROC_STAT_PATH: &str = "/proc/stat";
pub const CPU_METRIC_NAMESPACE: &str = "cpu";

pub struct CpuMetricCollector {
    last_reading_by_cpu: HashMap<String, Vec<f64>>,
}

impl CpuMetricCollector {
    pub fn new() -> Self {
        Self {
            last_reading_by_cpu: HashMap::new(),
        }
    }

    pub fn get_cpu_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        // Track if any lines in /proc/stat are parse-able
        // so we can alert user if none are
        let mut no_parseable_lines = true;

        let path = Path::new(PROC_STAT_PATH);

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut cpu_metric_readings = vec![];

        for line in reader.lines() {
            // Discard errors - the assumption here is that we are only parsing
            // lines that follow the specified format and expect other lines in the file to error
            if let Ok((cpu_id, cpu_stats)) = Self::parse_proc_stat_line_cpu(line?.trim()) {
                no_parseable_lines = false;
                if let Ok(Some(mut readings)) = self.delta_since_last_reading(cpu_id, cpu_stats) {
                    cpu_metric_readings.append(&mut readings);
                }
            }
        }

        // Check if we were able to parse at least one CPU metric reading
        if !no_parseable_lines {
            Ok(cpu_metric_readings)
        } else {
            Err(eyre!(
                    "No CPU metrics were collected from {} - is it a properly formatted /proc/stat file?",
                    PROC_STAT_PATH
            ))
        }
    }

    /// Parse a cpu ID from a line of /proc/stat
    ///
    /// A cpu ID is the digit following the 3 character string "cpu"
    /// No ID being present implies these stats are for the total
    /// of all cores on the CPU
    fn parse_cpu_id(input: &str) -> IResult<&str, &str> {
        preceded(tag("cpu"), digit0)(input)
    }

    /// Parse the CPU stats from the suffix of a /proc/stat line following the cpu ID
    ///
    /// 7 or more space delimited integers are expected. Values after the 7th are discarded.
    fn parse_cpu_stats(input: &str) -> IResult<&str, Vec<f64>> {
        count(preceded(space1, double), 7)(input)
    }

    /// Parse the output of a line of /proc/stat, returning
    /// a pair of the cpu ID that the parsed line corresponds
    /// to and the first 7 floats listed for it
    ///
    /// The 7 floats represent how much time since boot the cpu has
    /// spent in the "user", "nice", "system", "idle", "iowait", "irq",
    /// "softirq", in that order    
    /// Example of a valid parse-able line:
    /// cpu2 36675 176 11216 1552961 689 0 54
    fn parse_proc_stat_line_cpu(line: &str) -> Result<(String, Vec<f64>)> {
        let (_remaining, (cpu_id, cpu_stats)) =
            pair(Self::parse_cpu_id, Self::parse_cpu_stats)(line)
                .map_err(|_e| eyre!("Failed to parse CPU stats line: {}", line))?;
        Ok(("cpu".to_string().add(cpu_id), cpu_stats))
    }

    /// Calculate the time spent in each state for the
    /// provided CPU core since the last reading collected
    /// by the CpuMetricCollector
    ///
    /// Returns an Ok(None) if there is no prior reading
    /// to calculate a delta from.
    fn delta_since_last_reading(
        &mut self,
        cpu_id: String,
        cpu_stats: Vec<f64>,
    ) -> Result<Option<Vec<KeyedMetricReading>>> {
        // Check to make sure there was a previous reading to calculate a delta with
        if let Some(last_stats) = self
            .last_reading_by_cpu
            .insert(cpu_id.clone(), cpu_stats.clone())
        {
            let delta = cpu_stats
                .iter()
                .zip(last_stats)
                .map(|(current, previous)| current - previous);

            let cpu_states_with_ticks = zip(
                ["user", "nice", "system", "idle", "iowait", "irq", "softirq"],
                delta,
            )
            .collect::<Vec<(&str, f64)>>();

            let sum: f64 = cpu_states_with_ticks.iter().map(|(_k, v)| v).sum();
            let timestamp = Utc::now();

            let readings = cpu_states_with_ticks
                .iter()
                .map(|(key, value)| -> Result<KeyedMetricReading, ErrReport> {
                    Ok(KeyedMetricReading::new(
                        MetricStringKey::from_str(&format!(
                            "{}/{}/percent/{}",
                            CPU_METRIC_NAMESPACE, cpu_id, key
                        ))
                        .map_err(|e| eyre!(e))?,
                        MetricReading::Histogram {
                            // Transform raw tick value to a percentage
                            value: 100.0 * value / sum,
                            timestamp,
                        },
                    ))
                })
                .collect::<Result<Vec<KeyedMetricReading>>>();
            match readings {
                Ok(readings) => Ok(Some(readings)),
                Err(e) => Err(e),
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {

    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("cpu 1000 5 0 0 2 0 0", "test_basic_line")]
    #[case("cpu0 1000 5 0 0 2 0 0 0 0 0", "test_basic_line_with_extra")]
    fn test_process_valid_proc_stat_line(#[case] proc_stat_line: &str, #[case] test_name: &str) {
        assert_json_snapshot!(test_name, 
                              CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line).unwrap(), 
                              {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
    }

    #[rstest]
    #[case("cpu 1000 5 0 0 2")]
    #[case("1000 5 0 0 2 0 0 0 0 0")]
    #[case("processor0 1000 5 0 0 2 0 0 0 0 0")]
    #[case("softirq 403453672 10204651 21667771 199 12328940 529390 0 3519783 161759969 147995 193294974")]
    fn test_fails_on_invalid_proc_stat_line(#[case] proc_stat_line: &str) {
        assert!(CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line).is_err())
    }

    #[rstest]
    #[case(
        "cpu 1000 5 0 0 2 0 0",
        "cpu 1500 20 4 1 2 0 0",
        "cpu 1550 200 40 3 3 0 0",
        "basic_delta"
    )]
    fn test_cpu_metric_collector_calcs(
        #[case] proc_stat_line_a: &str,
        #[case] proc_stat_line_b: &str,
        #[case] proc_stat_line_c: &str,
        #[case] test_name: &str,
    ) {
        let mut cpu_metric_collector = CpuMetricCollector::new();

        let (cpu, stats) = CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line_a).unwrap();
        let result_a = cpu_metric_collector.delta_since_last_reading(cpu, stats);
        matches!(result_a, Ok(None));

        let (cpu, stats) = CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line_b).unwrap();
        let result_b = cpu_metric_collector.delta_since_last_reading(cpu, stats);

        assert!(result_b.is_ok());

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "a_b_metrics"),
                                  result_b.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });

        let (cpu, stats) = CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line_c).unwrap();
        let result_c = cpu_metric_collector.delta_since_last_reading(cpu, stats);

        assert!(result_c.is_ok());

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "b_c_metrics"),
                                  result_c.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }

    #[rstest]
    #[case(
        "cpu1 40 20 30 10 0 0 0",
        "cpu0 1500 20 4 1 2 0 0",
        "cpu1 110 30 40 12 5 3 0",
        "different_cores"
    )]
    fn test_cpu_metric_collector_different_cores(
        #[case] proc_stat_line_a: &str,
        #[case] proc_stat_line_b: &str,
        #[case] proc_stat_line_c: &str,
        #[case] test_name: &str,
    ) {
        let mut cpu_metric_collector = CpuMetricCollector::new();

        let (cpu, stats) = CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line_a).unwrap();
        let result_a = cpu_metric_collector.delta_since_last_reading(cpu, stats);
        matches!(result_a, Ok(None));

        let (cpu, stats) = CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line_b).unwrap();
        let result_b = cpu_metric_collector.delta_since_last_reading(cpu, stats);
        matches!(result_b, Ok(None));

        let (cpu, stats) = CpuMetricCollector::parse_proc_stat_line_cpu(proc_stat_line_c).unwrap();
        let result_c = cpu_metric_collector.delta_since_last_reading(cpu, stats);

        assert!(result_c.is_ok());

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "a_c_metrics"),
                                  result_c.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }
}
