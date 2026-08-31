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
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    iter::zip,
    path::Path,
    str::FromStr,
};

use chrono::Utc;
use log::debug;
use nom::{
    bytes::complete::tag,
    character::complete::{space1, u64},
    multi::count,
    sequence::preceded,
    IResult,
};

use crate::{
    metrics::{
        core_metrics::METRIC_CPU_USAGE_PCT, system_metrics::SystemMetricFamilyCollector,
        KeyedMetricReading, MetricReading, MetricStringKey,
    },
    util::{math::counter_delta_with_overflow, time_measure::TimeMeasure},
};
use eyre::{eyre, ErrReport, Result};

const PROC_STAT_PATH: &str = "/proc/stat";
pub const CPU_METRIC_NAMESPACE: &str = "cpu";
pub const CTXT_PER_SEC: &str = "cpu/context_switches_per_second";
pub const FORK_PER_SEC: &str = "cpu/forks_per_second";

#[derive(Debug, Clone, Copy)]
struct SingleStat<T>
where
    T: TimeMeasure + Copy,
{
    pub counter: u64,
    pub reading_time: T,
}
impl<T> SingleStat<T>
where
    T: TimeMeasure + Copy,
{
    pub fn rate_between(&self, other: Self) -> f64 {
        let counter_delta = counter_delta_with_overflow(self.counter, other.counter);
        let interval_secs = self.reading_time.since(&other.reading_time).as_secs_f64();
        if interval_secs <= 0.0 {
            return 0.0;
        }
        (counter_delta as f64) / interval_secs
    }
}
type CtxtStat<T> = SingleStat<T>;
type ProcessesStat<T> = SingleStat<T>;

#[derive(Debug, Clone)]
pub struct CpuMetricCollector<T>
where
    T: TimeMeasure + Copy,
{
    last_cpu_reading: Option<Vec<u64>>,
    last_ctxt_reading: Option<CtxtStat<T>>,
    last_processes_reading: Option<ProcessesStat<T>>,
}

impl<T> CpuMetricCollector<T>
where
    T: TimeMeasure + Copy,
{
    pub fn new() -> Self {
        Self {
            last_cpu_reading: None,
            last_ctxt_reading: None,
            last_processes_reading: None,
        }
    }

    pub fn get_cpu_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        // Track if any lines in /proc/stat are parse-able
        // so we can alert user if none are
        let mut no_parseable_lines = true;

        let path = Path::new(PROC_STAT_PATH);

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let reading_time = T::now();

        let mut cpu_metric_readings = vec![];
        for line in reader.lines().map_while(Result::ok) {
            let line = line.trim();
            if let Ok(cpu_stats) = Self::parse_proc_stat_line_cpu(line) {
                no_parseable_lines = false;
                if let Ok(Some(mut readings)) = self.cpu_delta_since_last_reading(cpu_stats) {
                    cpu_metric_readings.append(&mut readings);
                }
            }
            if let Ok(ctxt_counter) = Self::parse_proc_stat_line_ctxt(line) {
                no_parseable_lines = false;
                if let Some(reading) = self.ctxt_delta_since_last_reading(CtxtStat {
                    counter: ctxt_counter,
                    reading_time,
                }) {
                    cpu_metric_readings.push(reading);
                }
            }
            if let Ok(counter) = Self::parse_proc_stat_line_processes(line) {
                no_parseable_lines = false;
                if let Some(reading) = self.processes_delta_since_last_reading(ProcessesStat {
                    counter,
                    reading_time,
                }) {
                    cpu_metric_readings.push(reading);
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

    /// Parse the CPU stats from the suffix of a /proc/stat line following the cpu ID
    ///
    /// 7 or more space delimited integers are expected. Values after the 7th are discarded.
    fn parse_cpu_stats(input: &str) -> IResult<&str, Vec<u64>> {
        preceded(tag("cpu"), count(preceded(space1, u64), 7))(input)
    }

    /// Parse the CPU stats from the suffix of a /proc/stat line following the `ctxt` tag
    fn parse_ctxt_stats(input: &str) -> IResult<&str, u64> {
        preceded(tag("ctxt"), preceded(space1, u64))(input)
    }
    /// Parse the CPU stats from the suffix of a /proc/stat line following the `processes` tag
    fn parse_processes_stats(input: &str) -> IResult<&str, u64> {
        preceded(tag("processes"), preceded(space1, u64))(input)
    }

    /// Parse the output of a line of /proc/stat, returning
    /// the first 7 floats listed for `cpu`
    ///
    /// The 7 floats represent how much time since boot the cpu has
    /// spent in the "user", "nice", "system", "idle", "iowait", "irq",
    /// "softirq", in that order
    ///
    /// Example of a valid parse-able line:
    ///
    /// cpu 36675 176 11216 1552961 689 0 54
    fn parse_proc_stat_line_cpu(line: &str) -> Result<Vec<u64>> {
        let (_, cpu_stats) = Self::parse_cpu_stats(line)
            .map_err(|_e| eyre!("Failed to parse CPU stats line: {}", line))?;
        Ok(cpu_stats)
    }
    /// Parse the output of a line of /proc/stat tagged with `ctxt`.
    /// Returned `u64` indicates reported number.
    ///
    /// Example of a valid parse-able line:
    ///
    /// ctxt 1235748
    fn parse_proc_stat_line_ctxt(line: &str) -> Result<u64> {
        let (_, ctxt_stat) = Self::parse_ctxt_stats(line)
            .map_err(|_e| eyre!("Failed to parse CPU ctxt line: {}", line))?;
        Ok(ctxt_stat)
    }
    /// Parse the output of a line of /proc/stat tagged with `processes`.
    /// Returned `u64` indicates reported number.
    ///
    /// Example of a valid parse-able line:
    ///
    /// processes 1237587
    fn parse_proc_stat_line_processes(line: &str) -> Result<u64> {
        let (_, stat) = Self::parse_processes_stats(line)
            .map_err(|_e| eyre!("Failed to parse CPU processes line: {}", line))?;
        Ok(stat)
    }

    fn ctxt_delta_since_last_reading(
        &mut self,
        current_stat: CtxtStat<T>,
    ) -> Option<KeyedMetricReading> {
        self.last_ctxt_reading
            .replace(current_stat)
            .map(|last_stat| {
                KeyedMetricReading::new_histogram(
                    MetricStringKey::from(CTXT_PER_SEC),
                    current_stat.rate_between(last_stat),
                )
            })
    }
    fn processes_delta_since_last_reading(
        &mut self,
        current_stat: ProcessesStat<T>,
    ) -> Option<KeyedMetricReading> {
        self.last_processes_reading
            .replace(current_stat)
            .map(|last_stat| {
                KeyedMetricReading::new_histogram(
                    MetricStringKey::from(FORK_PER_SEC),
                    current_stat.rate_between(last_stat),
                )
            })
    }

    /// Calculate the time spent in each state for the
    /// provided CPU core since the last reading collected
    /// by the CpuMetricCollector
    ///
    /// Returns an Ok(None) if there is no prior reading
    /// to calculate a delta from.
    fn cpu_delta_since_last_reading(
        &mut self,
        cpu_stats: Vec<u64>,
    ) -> Result<Option<Vec<KeyedMetricReading>>> {
        // Check to make sure there was a previous reading to calculate a delta with
        if let Some(last_stats) = self.last_cpu_reading.replace(cpu_stats.clone()) {
            // TODO: probably want to remove this clone?
            let delta = cpu_stats
                .iter()
                .zip(last_stats)
                .map(|(current, previous)| counter_delta_with_overflow(*current, previous));

            let cpu_states_with_ticks = zip(
                ["user", "nice", "system", "idle", "iowait", "irq", "softirq"],
                delta,
            )
            .collect::<HashMap<&str, u64>>();

            let sum: f64 = cpu_states_with_ticks.values().sum::<u64>() as f64;
            let timestamp = Utc::now();

            let readings = cpu_states_with_ticks
                .iter()
                .map(|(key, value)| -> Result<KeyedMetricReading, ErrReport> {
                    Ok(KeyedMetricReading::new(
                        MetricStringKey::from_str(&format!(
                            "{}/cpu/percent/{}",
                            CPU_METRIC_NAMESPACE, key
                        ))
                        .map_err(|e| eyre!(e))?,
                        MetricReading::Histogram {
                            // Transform raw tick value to a percentage
                            value: 100.0 * *value as f64 / sum,
                            timestamp,
                        },
                    ))
                })
                .collect::<Result<Vec<KeyedMetricReading>>>()?;

            if sum > 0.0 {
                let _cpu_usage_pct = ((sum - cpu_states_with_ticks["idle"] as f64) / sum) * 100.0;
                let _cpu_usage_pct_key =
                    MetricStringKey::from_str(METRIC_CPU_USAGE_PCT).map_err(|e| {
                        eyre!("Failed to construct MetricStringKey for used memory: {}", e)
                    })?;
            } else {
                debug!("Sum of time spent in all CPU states is <= 0 - this is probably incorrect.")
            }

            Ok(Some(readings))
        } else {
            Ok(None)
        }
    }
}

impl<T> SystemMetricFamilyCollector for CpuMetricCollector<T>
where
    T: TimeMeasure + Copy + Send,
{
    fn family_name(&self) -> &'static str {
        CPU_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_cpu_metrics()
    }
}

#[cfg(test)]
mod test {

    use std::time::Duration;

    use crate::test_utils::TestInstant;
    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("cpu 1000 5 0 0 2 0 0", "test_basic_line")]
    fn test_process_valid_cpu_proc_stat_line(#[case] cpu_stat_line: &str, #[case] test_name: &str) {
        assert_json_snapshot!(test_name,
                              CpuMetricCollector::<TestInstant>::parse_proc_stat_line_cpu(cpu_stat_line).unwrap(),
                              {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
    }

    #[rstest]
    #[case("cpu 1000 5 0 0 2")]
    #[case("1000 5 0 0 2 0 0 0 0 0")]
    #[case("processor0 1000 5 0 0 2 0 0 0 0 0")]
    #[case("softirq 403453672 10204651 21667771 199 12328940 529390 0 3519783 161759969 147995 193294974")]
    fn test_fails_on_invalid_cpu_proc_stat_line(#[case] cpu_stat_line: &str) {
        assert!(CpuMetricCollector::<TestInstant>::parse_proc_stat_line_cpu(cpu_stat_line).is_err())
    }

    #[rstest]
    #[case("ctxt 12345", 12345)]
    #[case("ctxt 127987", 127987)]
    fn test_process_valid_ctxt_proc_stat_line(#[case] ctxt_stat_line: &str, #[case] expected: u64) {
        let (_, res) = CpuMetricCollector::<TestInstant>::parse_ctxt_stats(ctxt_stat_line)
            .expect("parsed a valid ctxt stat");
        assert_eq!(res, expected);
    }
    #[rstest]
    #[case("ctxt ")]
    #[case("asdf")]
    #[case("processes 12345")]
    #[case("cpu 1000 5 0 0 2 0 0")]
    fn test_fails_on_invalid_ctxt_proc_stat_line(#[case] cpu_stat_line: &str) {
        assert!(CpuMetricCollector::<TestInstant>::parse_ctxt_stats(cpu_stat_line).is_err())
    }
    #[rstest]
    #[case("processes 12345", 12345)]
    #[case("processes 127987", 127987)]
    fn test_process_valid_processes_proc_stat_line(
        #[case] processes_stat_line: &str,
        #[case] expected: u64,
    ) {
        let (_, res) =
            CpuMetricCollector::<TestInstant>::parse_processes_stats(processes_stat_line)
                .expect("parsed a valid ctxt stat");
        assert_eq!(res, expected);
    }
    #[rstest]
    #[case("processes ")]
    #[case("asdf")]
    #[case("ctxt 12345")]
    #[case("cpu 1000 5 0 0 2 0 0")]
    fn test_fails_on_invalid_processes_proc_stat_line(#[case] cpu_stat_line: &str) {
        assert!(CpuMetricCollector::<TestInstant>::parse_processes_stats(cpu_stat_line).is_err())
    }

    #[rstest]
    #[case("ctxt 0", 0, "ctxt 10", 10, 10, 1.0f64)]
    #[case("ctxt 10", 10, "ctxt 12", 12, 4, 0.5f64)]
    #[case("ctxt 300", 300, "ctxt 301", 301, 1, 1.0f64)]
    #[case(
        "ctxt 18446744073709551615",
        0xFFFFFFFFFFFFFFFF,
        "ctxt 2",
        2,
        1,
        2.0f64
    )]
    fn test_ctxt_metrics_calculation(
        #[case] before_line: &str,
        #[case] expected_before: u64,
        #[case] after_line: &str,
        #[case] expected_after: u64,
        #[case] sleep_secs: u64,
        #[case] expected_diff: f64,
    ) {
        let res_before = CpuMetricCollector::<TestInstant>::parse_proc_stat_line_ctxt(before_line)
            .expect("valid before_line");
        assert_eq!(expected_before, res_before);
        let res_after = CpuMetricCollector::<TestInstant>::parse_proc_stat_line_ctxt(after_line)
            .expect("valid before_line");
        assert_eq!(expected_after, res_after);

        let mut collector = CpuMetricCollector::<TestInstant>::new();

        let delta1 = collector.ctxt_delta_since_last_reading(CtxtStat {
            counter: res_before,
            reading_time: TestInstant::from(Duration::from_secs(0)),
        });

        assert!(delta1.is_none());

        let delta2 = collector
            .ctxt_delta_since_last_reading(CtxtStat {
                counter: res_after,
                reading_time: TestInstant::from(Duration::from_secs(sleep_secs)),
            })
            .expect("delta 2 should be some value");

        match delta2.value {
            MetricReading::Histogram { value, .. } => assert_eq!(expected_diff, value),
            _ => panic!("unexpected variant: {:#?}", delta2.value),
        }
    }

    #[test]
    fn test_processes_metrics_calculation() {
        let mut collector = CpuMetricCollector::<TestInstant>::new();

        let delta1 = collector.processes_delta_since_last_reading(ProcessesStat {
            counter: 0,
            reading_time: TestInstant::from(Duration::from_secs(0)),
        });
        assert!(delta1.is_none());

        let delta2 = collector
            .processes_delta_since_last_reading(ProcessesStat {
                counter: 10,
                reading_time: TestInstant::from(Duration::from_secs(10)),
            })
            .expect("delta 2 should be some value");

        match delta2.value {
            MetricReading::Histogram { value, .. } => assert_eq!(1.0, value),
            _ => panic!("unexpected variant: {:#?}", delta2.value),
        }
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
        let mut cpu_metric_collector = CpuMetricCollector::<TestInstant>::new();

        let stats =
            CpuMetricCollector::<TestInstant>::parse_proc_stat_line_cpu(proc_stat_line_a).unwrap();
        let result_a = cpu_metric_collector.cpu_delta_since_last_reading(stats);
        matches!(result_a, Ok(None));

        let stats =
            CpuMetricCollector::<TestInstant>::parse_proc_stat_line_cpu(proc_stat_line_b).unwrap();
        let mut result_b = cpu_metric_collector
            .cpu_delta_since_last_reading(stats)
            .unwrap()
            .unwrap();
        result_b.sort_by(|a, b| a.name.cmp(&b.name));

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "a_b_metrics"),
                                  result_b,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });

        let stats =
            CpuMetricCollector::<TestInstant>::parse_proc_stat_line_cpu(proc_stat_line_c).unwrap();
        let mut result_c = cpu_metric_collector
            .cpu_delta_since_last_reading(stats)
            .unwrap()
            .unwrap();
        result_c.sort_by(|a, b| a.name.cmp(&b.name));

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "b_c_metrics"),
                                  result_c,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }
}
