//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect per-process metrics from /proc/<pid>/stat
//!
//! Collects process-level metrics using the
//! /proc/<pid>/stat files for processes whose
//! process name matches an item in the user-specified
//! list of processes to monitor.
//!
//! Example /proc/<pid>/stat contents:
//!
//!   55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 155 102 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0
//!
//! Further documentation of the /proc/<pid>/stat file
//! can be found at:
//! https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html
use std::{
    collections::HashMap,
    collections::HashSet,
    fs::{read_dir, read_to_string},
    marker::PhantomData,
};

use eyre::{eyre, Result};
use log::warn;
use nom::character::complete::{alpha1, multispace1};
use nom::{
    bytes::complete::{is_not, tag},
    character::complete::{space1, u64},
    multi::count,
    number::complete::double,
    sequence::{delimited, preceded, terminated},
    IResult,
};

use crate::metrics::{system_metrics::SystemMetricFamilyCollector, KeyedMetricReading};
use crate::util::{
    math::counter_delta_with_overflow, system::ProcessNameMapper, time_measure::TimeMeasure,
};

const PROC_DIR: &str = "/proc/";
pub const PROCESSES_METRIC_NAMESPACE: &str = "processes";

#[derive(Clone)]
pub enum ProcessMetricsConfig {
    Auto,
    Processes(HashSet<String>),
}

#[derive(Clone, Debug)]
struct ProcessReading<T: TimeMeasure> {
    pid: u64,
    name: String,
    cputime_user: f64,
    cputime_system: f64,
    num_threads: f64,
    rss: f64,
    vm: f64,
    pagefaults_major: f64,
    pagefaults_minor: f64,
    reading_time: T,
}

pub struct ProcessMetricsCollector<T: TimeMeasure, P: ProcessNameMapper> {
    config: ProcessMetricsConfig,
    processes: HashMap<u64, ProcessReading<T>>,
    clock_ticks_per_ms: f64,
    bytes_per_page: f64,
    mem_total: f64,
    _marker: PhantomData<P>,
}

impl<T, P> ProcessMetricsCollector<T, P>
where
    T: TimeMeasure + Copy + Send + Sync + 'static,
    P: ProcessNameMapper + Copy + Send + Sync,
{
    pub fn new(
        config: ProcessMetricsConfig,
        clock_ticks_per_ms: f64,
        bytes_per_page: f64,
        mem_total: f64,
    ) -> Self {
        Self {
            config,
            processes: HashMap::new(),
            clock_ticks_per_ms,
            bytes_per_page,
            mem_total,
            _marker: PhantomData,
        }
    }

    fn process_is_monitored(&self, process_name: &str) -> bool {
        match &self.config {
            ProcessMetricsConfig::Auto => process_name == "memfaultd",
            ProcessMetricsConfig::Processes(ps) => ps.contains(process_name),
        }
    }

    /// Parses the PID at the start of the /proc/<pid>/stat
    ///
    /// Example input:
    /// 55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
    /// Example output:
    /// 55270
    fn parse_pid(proc_pid_stat_line: &str) -> IResult<&str, u64> {
        terminated(u64, space1)(proc_pid_stat_line)
    }

    /// Parses the process name which follows the PID, delimited by ( and ), in /proc/<pid>/stat
    ///
    /// Note this snippet from the documentation on /proc/<pid>/stat:
    ///
    /// (2) comm  %s
    ///        The filename of the executable, in parentheses.
    ///        Strings longer than TASK_COMM_LEN (16) characters
    ///        (including the terminating null byte) are silently
    ///        truncated.  This is visible whether or not the
    ///        executable is swapped out.
    ///
    /// So for executables with names longer than 16 characters, the
    /// name will be truncated to just the first 16 characters.
    ///
    /// Example input:
    /// (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
    /// Example output:
    /// memfaultd
    fn parse_comm(proc_pid_stat_line: &str) -> IResult<&str, &str> {
        // This will break if there's a ')' in the process name - that seems unlikely
        // enough to leave as-is for now
        delimited(tag("("), is_not(")"), tag(")"))(proc_pid_stat_line)
    }

    /// Parses the process state which follows the PID, delimited by ( and )
    ///
    /// Example input:
    ///  S 1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
    /// Example output:
    /// S
    fn parse_state(proc_pid_stat_line: &str) -> IResult<&str, &str> {
        preceded(space1, alpha1)(proc_pid_stat_line)
    }

    /// Parses the process state which follows the PID, delimited by ( and )
    ///
    /// The following values from the resulting Vector are currently used:
    /// - minfault: The number of minor faults the process has made
    ///             which have not required loading a memory page from
    ///             disk. (index 6)
    /// - majfault: The number of major faults the process has made
    ///             which have required loading a memory page from
    ///             disk. (index 8)
    /// - utime: Amount of time that this process has been scheduled
    ///          in user mode, measured in clock ticks (index 10)
    /// - stime: Amount of time that this process has been scheduled
    ///          in kernel mode, measured in clock ticks (index 11)
    /// - num_threads: Number of threads in the corresponding process (Index 16)
    /// - vsize: Virtual memory size in bytes for the process (index 19)
    /// - rss: number of pages the process has in real memory (index 20)
    ///
    /// Example input:
    ///  1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
    /// Example output:
    /// vec![1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0]
    fn parse_stats(proc_pid_stat_line: &str) -> IResult<&str, Vec<f64>> {
        // There are more than 29 values in a line but we don't use any past the 29th
        // (kstkesp) so don't parse past that
        count(preceded(multispace1, double), 29)(proc_pid_stat_line)
    }

    /// Parses the full contents of a /proc/<pid>/stat file into a ProcessReading
    ///
    /// If the process name is not in the set of configured processes to monitor, this
    /// function will stop parsing and return Ok(None) to avoid doing unnecessary work.
    fn parse_process_stat(&self, proc_pid_stat_line: &str) -> Result<Option<ProcessReading<T>>> {
        let (after_pid, pid) = Self::parse_pid(proc_pid_stat_line)
            .map_err(|_e| eyre!("Failed to parse PID for process"))?;
        let (after_comm, _comm) = Self::parse_comm(after_pid)
            .map_err(|_e| eyre!("Failed to parse process comm value"))?;

        let name = P::get_process_name(pid as u32)?;

        // Don't bother continuing to parse processes that aren't monitored
        if self.process_is_monitored(&name) {
            let (after_state, _) = Self::parse_state(after_comm)
                .map_err(|_e| eyre!("Failed to parse process state for {}", name))?;
            let (_, stats) = Self::parse_stats(after_state)
                .map_err(|_e| eyre!("Failed to parse process stats for {}", name))?;

            let pagefaults_minor = *stats
                .get(6)
                .ok_or(eyre!("Failed to read pagefaults_minor"))?;
            let pagefaults_major = *stats
                .get(8)
                .ok_or(eyre!("Failed to read pagefaults_major"))?;

            let cputime_user = *stats.get(10).ok_or(eyre!("Failed to read cputime_user"))?;
            let cputime_system = *stats.get(11).ok_or(eyre!("Failed to read cputime_user"))?;

            let num_threads = *stats.get(16).ok_or(eyre!("Failed to read num_threads"))?;

            let vm = *stats.get(19).ok_or(eyre!("Failed to read vm"))?;

            // RSS is provided as the number of pages used by the process, we need
            // to multiply by the system-specific bytes per page to get a value in bytes
            let rss = *stats.get(20).ok_or(eyre!("Failed to read rss"))? * self.bytes_per_page;

            Ok(Some(ProcessReading {
                pid,
                name,
                cputime_user,
                cputime_system,
                num_threads,
                rss,
                pagefaults_major,
                pagefaults_minor,
                vm,
                reading_time: T::now(),
            }))
        } else {
            Ok(None)
        }
    }

    fn calculate_metric_readings(
        &self,
        previous: ProcessReading<T>,
        current: ProcessReading<T>,
    ) -> Result<Vec<KeyedMetricReading>> {
        let rss_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/rss_bytes", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            current.rss,
        );

        let vm_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/vm_bytes", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            current.vm,
        );

        let num_threads_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/num_threads", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            current.num_threads,
        );

        // The values from /proc/<pid>/stat are monotonic counters of jiffies since the
        // process was started. We need to calculate the difference since the last reading,
        // divide by the amount of jiffies in a millisecond (to get milliseconds spent in
        // the given state), then divide by the total number of milliseconds since the
        // previous reading in order to give us the % of time this process caused
        // the CPU to spend in the user and system states.
        let cputime_user_pct = ((counter_delta_with_overflow(
            current.cputime_user as u64,
            previous.cputime_user as u64,
        ) as f64
            / self.clock_ticks_per_ms)
            / (current
                .reading_time
                .since(&previous.reading_time)
                .as_millis() as f64))
            * 100.0;

        let cputime_sys_pct = ((counter_delta_with_overflow(
            current.cputime_system as u64,
            previous.cputime_system as u64,
        ) as f64
            / self.clock_ticks_per_ms)
            / (current
                .reading_time
                .since(&previous.reading_time)
                .as_millis() as f64))
            * 100.0;

        let utime_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/cpu/percent/user", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            cputime_user_pct,
        );
        let stime_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/cpu/percent/system", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            cputime_sys_pct,
        );

        let pagefaults_minor_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/pagefaults/minor", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            counter_delta_with_overflow(
                current.pagefaults_minor as u64,
                previous.pagefaults_minor as u64,
            ) as f64,
        );
        let pagefaults_major_reading = KeyedMetricReading::new_histogram(
            format!("processes/{}/pagefaults/major", current.name)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
            counter_delta_with_overflow(
                current.pagefaults_major as u64,
                previous.pagefaults_major as u64,
            ) as f64,
        );

        let _cpu_usage_process_pct = cputime_sys_pct + cputime_user_pct;
        let _memory_process_pct = current.rss / self.mem_total;

        Ok(vec![
            rss_reading,
            vm_reading,
            num_threads_reading,
            stime_reading,
            utime_reading,
            pagefaults_minor_reading,
            pagefaults_major_reading,
        ])
    }

    // To facilitate unit testing, make the process directory path an arg
    fn read_process_metrics_from_dir(&mut self, proc_dir: &str) -> Result<Vec<KeyedMetricReading>> {
        let process_readings: Vec<_> = read_dir(proc_dir)?
            .filter_map(|entry| entry.map(|e| e.path()).ok())
            // Filter out non-numeric directories (since these won't be PIDs)
            .filter(|path| match path.file_name() {
                Some(p) => p.to_string_lossy().chars().all(|c| c.is_numeric()),
                None => false,
            })
            // Append "/stat" to the path since this is the file we want to read
            // for a given PID's directory
            .filter_map(|path| read_to_string(path.join("stat")).ok())
            .filter_map(|proc_pid_stat_contents| {
                self.parse_process_stat(&proc_pid_stat_contents).ok()
            })
            .flatten()
            .collect();

        let mut process_metric_readings = vec![];
        for current_reading in process_readings {
            // A previous reading is required to calculate CPU time %s, as
            // /proc/<pid>/stat only has monotonic counters that track
            // time spent in CPU states. Without a delta over a known period
            // of time, we can't know if the contents of the counter are relevant
            // to the current sampling window or not.
            //
            // For simplicity, only return any metrics for a PID when there
            // is a previous reading for that PID in the `processes` map.
            if let Some(previous_reading) = self
                .processes
                .insert(current_reading.pid, current_reading.clone())
            {
                match self.calculate_metric_readings(previous_reading, current_reading.clone()) {
                    Ok(metric_readings) => process_metric_readings.extend(metric_readings),
                    Err(e) => warn!(
                        "Couldn't calculate metric readings for process {} (PID {}): {}",
                        current_reading.name, current_reading.pid, e
                    ),
                }
            }
        }

        Ok(process_metric_readings)
    }

    pub fn get_process_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.read_process_metrics_from_dir(PROC_DIR)
    }
}

impl<T, P> SystemMetricFamilyCollector for ProcessMetricsCollector<T, P>
where
    T: TimeMeasure + Copy + Send + Sync + 'static,
    P: ProcessNameMapper + Copy + Send + Sync,
{
    fn family_name(&self) -> &'static str {
        PROCESSES_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_process_metrics()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{create_dir, remove_file, File},
        io::Write,
        time::Duration,
    };
    use tempfile::tempdir;

    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;

    use super::*;
    use crate::test_utils::TestInstant;

    #[derive(Copy, Clone)]
    struct MockProcessNameMapper {}
    impl ProcessNameMapper for MockProcessNameMapper {
        fn get_process_name(pid: u32) -> Result<String> {
            match pid {
                55270 => Ok("memfaultd".to_string()),
                24071 => Ok("systemd".to_string()),
                _ => Err(eyre!("Unmapped PID in test case")),
            }
        }
    }

    #[rstest]
    fn test_parse_single_line() {
        let collector = ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::new(
            ProcessMetricsConfig::Processes(HashSet::from_iter(["memfaultd".to_string()])),
            100.0,
            4096.0,
            1000000000.0,
        );

        let line = "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 155 102 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert!(
            ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::parse_process_stat(
                &collector, line
            )
            .is_ok()
        );
    }

    #[rstest]
    #[case(
        "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 875 0 10 0 1100 550 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "simple_cpu_delta",
    )]
    fn test_collect_metrics(#[case] line1: &str, #[case] line2: &str, #[case] test_name: &str) {
        let collector = ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::new(
            ProcessMetricsConfig::Processes(HashSet::from_iter(["memfaultd".to_string()])),
            100.0,
            4096.0,
            1000000000.0,
        );

        let first_reading =
            ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::parse_process_stat(
                &collector, line1,
            )
            .unwrap()
            .unwrap();

        TestInstant::sleep(Duration::from_secs(10));

        let second_reading =
            ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::parse_process_stat(
                &collector, line2,
            )
            .unwrap()
            .unwrap();

        let process_metric_readings =
            collector.calculate_metric_readings(first_reading, second_reading);
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(format!("{}_{}", test_name, "metrics"),
                                  process_metric_readings.unwrap(),
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }

    #[rstest]
    #[case(
        "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "24071 (systemd) S 1 24071 24071 0 -1 4194560 1580 2275 0 0 12 2 0 1 20 0 1 0 1465472 19828736 2784 18446744073709551615 1 1 0 0 0 0 671173123 4096 0 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 845 0 16 0 1100 550 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "24071 (systemd) S 1 24071 24071 0 -1 4194560 1580 2275 0 0 100 30 0 1 20 0 1 0 1465472 19828736 2784 18446744073709551615 1 1 0 0 0 0 671173123 4096 0 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        false,
    )]
    #[case(
        "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 100 50 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "24071 (systemd) S 1 24071 24071 0 -1 4194560 1580 2275 0 0 12 2 0 1 20 0 1 0 1465472 19828736 2784 18446744073709551615 1 1 0 0 0 0 671173123 4096 0 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "55270 (memfaultd) S 1 55270 55270 0 -1 4194368 825 0 0 0 1100 550 0 0 20 0 19 0 18548522 1411293184 4397 18446744073709551615 1 1 0 0 0 0 0 4096 17987 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        "24071 (systemd) S 1 24071 24071 0 -1 4194560 1580 2275 0 0 100 30 0 1 20 0 1 0 1465472 19828736 2784 18446744073709551615 1 1 0 0 0 0 671173123 4096 0 0 0 0 17 7 0 0 0 0 0 0 0 0 0 0 0 0 0",
        true,
    )]
    fn test_process_stats_from_proc(
        #[case] process_a_sample_1: &str,
        #[case] process_b_sample_1: &str,
        #[case] process_a_sample_2: &str,
        #[case] process_b_sample_2: &str,
        #[case] use_auto: bool,
    ) {
        let mut collector = if use_auto {
            ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::new(
                ProcessMetricsConfig::Auto,
                100.0,
                4096.0,
                1000000000.0,
            )
        } else {
            // If auto is not used, the configuration should capture metrics from both processes
            ProcessMetricsCollector::<TestInstant, MockProcessNameMapper>::new(
                ProcessMetricsConfig::Processes(HashSet::from_iter([
                    "memfaultd".to_string(),
                    "systemd".to_string(),
                ])),
                100.0,
                4096.0,
                1000000000.0,
            )
        };

        // Create a temporary directory.
        let dir = tempdir().unwrap();

        let temp_proc_dir = dir.path().join("temp_proc");
        create_dir(&temp_proc_dir).unwrap();

        // Set up /proc/<pid>/stat files for first sample for both
        // memfaultd and systemd
        let process_a_dir = temp_proc_dir.join("55270");
        create_dir(&process_a_dir).unwrap();

        let process_a_path = process_a_dir.join("stat");
        let mut process_a_file = File::create(process_a_path.clone()).unwrap();

        let process_b_dir = temp_proc_dir.join("24071");
        create_dir(&process_b_dir).unwrap();
        let process_b_path = process_b_dir.join("stat");
        let mut process_b_file = File::create(process_b_path.clone()).unwrap();

        writeln!(process_a_file, "{}", process_a_sample_1).unwrap();
        writeln!(process_b_file, "{}", process_b_sample_1).unwrap();

        // Read /proc/<pid/stat files - since this is the first sample for
        // both processes no metric readings should be returned
        let process_metric_readings = collector
            .read_process_metrics_from_dir(temp_proc_dir.as_os_str().to_str().unwrap())
            .unwrap();

        assert!(process_metric_readings.is_empty());

        TestInstant::sleep(Duration::from_secs(10));
        // Clear files for first samples
        remove_file(process_a_path).unwrap();
        remove_file(process_b_path).unwrap();

        // Set up /proc/<pid>/stat files for second sample for both
        // memfaultd and systemd
        let process_a_path = temp_proc_dir.join("55270").join("stat");
        let mut process_a_file = File::create(process_a_path).unwrap();

        let process_b_path = temp_proc_dir.join("24071").join("stat");
        let mut process_b_file = File::create(process_b_path).unwrap();

        writeln!(process_a_file, "{}", process_a_sample_2).unwrap();
        writeln!(process_b_file, "{}", process_b_sample_2).unwrap();

        // Read /proc/<pid/stat files again - this time
        // metric readings should be returned
        let mut process_metric_readings_2 = collector
            .read_process_metrics_from_dir(temp_proc_dir.as_os_str().to_str().unwrap())
            .unwrap();

        process_metric_readings_2.sort_by(|a, b| a.name.cmp(&b.name));

        assert_json_snapshot!(format!("process_metrics_auto_{}", use_auto),
                                  process_metric_readings_2,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)});

        // Delete the temporary directory.
        dir.close().unwrap();
    }
}
