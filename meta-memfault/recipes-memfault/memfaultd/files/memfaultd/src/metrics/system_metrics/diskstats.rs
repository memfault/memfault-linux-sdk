//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect disk I/O metrics for disks listed in
//! /proc/diskstats
//!
//! Linux Kernel documentation on /proc/diskstats:
//! https://www.kernel.org/doc/Documentation/iostats.txt
//!

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    str::FromStr,
};

use nom::{
    character::complete::{alphanumeric1, multispace0, multispace1, u64},
    multi::count,
    sequence::preceded,
    IResult,
};

use crate::{
    metrics::{system_metrics::SystemMetricFamilyCollector, KeyedMetricReading, MetricStringKey},
    util::{math::counter_delta_with_overflow, time_measure::TimeMeasure},
};
use eyre::{eyre, Result};

const PROC_DISKSTATS_PATH: &str = "/proc/diskstats";
pub const DISKSTATS_METRIC_NAMESPACE: &str = "diskstats";

pub enum DiskstatsMetricsConfig {
    Auto,
    Devices(HashSet<String>),
}

struct DiskstatsReading<T: TimeMeasure + Clone> {
    num_reads: u64,
    num_writes: u64,
    reading_time: T,
}

pub struct DiskstatsMetricCollector<T: TimeMeasure + Clone> {
    config: DiskstatsMetricsConfig,
    disks: HashMap<String, DiskstatsReading<T>>,
}

impl<T> DiskstatsMetricCollector<T>
where
    T: TimeMeasure + Clone,
{
    pub fn new(config: DiskstatsMetricsConfig) -> Self {
        Self {
            config,
            disks: HashMap::new(),
        }
    }

    pub fn get_diskstats_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        // Track if any lines in /proc/diskstats are parse-able
        // so we can alert user if none are
        let mut no_parseable_lines = true;

        let path = Path::new(PROC_DISKSTATS_PATH);

        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut diskstat_metric_readings = vec![];

        for line in reader.lines() {
            // Discard errors - the assumption here is that we are only parsing
            // lines that follow the specified format and expect other lines in the file to error
            if let Ok((device_name, disk_stats)) = Self::parse_proc_diskstats_line(line?.trim()) {
                no_parseable_lines = false;

                if self.device_is_monitored(&device_name) {
                    if let Ok(Some(mut readings)) =
                        self.delta_since_last_reading(device_name, disk_stats)
                    {
                        diskstat_metric_readings.append(&mut readings);
                    }
                }
            }
        }

        if !no_parseable_lines {
            Ok(diskstat_metric_readings)
        } else {
            Err(eyre!(
                "No diskstats metrics were collected from {} - is it a properly formatted /proc/diskstats file?",
                PROC_DISKSTATS_PATH,
            ))
        }
    }

    fn device_is_monitored(&self, device_name: &str) -> bool {
        match &self.config {
            DiskstatsMetricsConfig::Devices(devices) => devices.contains(device_name),
            DiskstatsMetricsConfig::Auto => {
                !(device_name.starts_with("loop") || device_name.starts_with("ram"))
            }
        }
    }

    /// Parses the disk stats from the content of a line of /proc/diskstats
    /// following the disk ID
    ///
    /// Example input:
    ///  57839 46346 3776180 24107 2868024 458991 39327918 1458693 0 839132 1568407 285564 2 308337155 49208 141738 36397
    ///
    /// Parsed to [57839, 46346, 3776180, 24107, 2868024 458991, 39327918, 1458693]
    fn parse_disk_stats(input: &str) -> IResult<&str, Vec<u64>> {
        count(preceded(multispace1, u64), 8)(input)
    }

    /// Parses the disk ID str from a line of /proc/diskstats
    ///
    /// Example input:
    ///    8       0 sda 57839 46346 3776180 24107 2868024 458991 39327918 1458693 0 839132 1568407 285564 2 308337155 49208 141738 36397
    ///
    /// Parsed to "sda", with everything after the disk ID returned as the other str
    /// in the IResult
    fn parse_device_name(input: &str) -> IResult<&str, &str> {
        // Use multispace0 here as there is not guaranteed to be a leading
        // space depending on the number of digits in the device's major number
        let (rest, _first_int) = preceded(multispace0, u64)(input)?;
        let (rest, _second_int) = preceded(multispace1, u64)(rest)?;
        preceded(multispace1, alphanumeric1)(rest)
    }

    fn parse_proc_diskstats_line(line: &str) -> Result<(String, Vec<u64>)> {
        let (stats_str, device_name) =
            Self::parse_device_name(line).map_err(|e| eyre!("Failed to parse disk ID: {}", e))?;
        let (_, disk_stats) = Self::parse_disk_stats(stats_str)
            .map_err(|e| eyre!("Failed to parse disk stats: {}", e))?;
        Ok((device_name.to_string(), disk_stats))
    }

    fn delta_since_last_reading(
        &mut self,
        device_name: String,
        diskstats: Vec<u64>,
    ) -> Result<Option<Vec<KeyedMetricReading>>> {
        let num_reads = diskstats.first().ok_or(eyre!("No num_reads"))?;
        let num_writes = diskstats.get(4).ok_or(eyre!("No num_writes"))?;
        let now = T::now();
        // Check to make sure there was a previous reading to calculate a delta with
        if let Some(last_stats) = self.disks.insert(
            device_name.clone(),
            DiskstatsReading {
                num_reads: *num_reads,
                num_writes: *num_writes,
                reading_time: now.clone(),
            },
        ) {
            let reads_per_second = counter_delta_with_overflow(*num_reads, last_stats.num_reads)
                as f64
                / now.since(&last_stats.reading_time).as_secs_f64();
            let writes_per_second = counter_delta_with_overflow(*num_writes, last_stats.num_writes)
                as f64
                / now.since(&last_stats.reading_time).as_secs_f64();

            Ok(Some(vec![
                KeyedMetricReading::new_histogram(
                    MetricStringKey::from_str(
                        format!("diskstats/{}/reads_per_second", device_name).as_str(),
                    )
                    .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
                    reads_per_second,
                ),
                KeyedMetricReading::new_histogram(
                    MetricStringKey::from_str(
                        format!("diskstats/{}/writes_per_second", device_name).as_str(),
                    )
                    .map_err(|e| eyre!("Couldn't parse metric key: {}", e))?,
                    writes_per_second,
                ),
            ]))
        } else {
            Ok(None)
        }
    }
}

impl<T> SystemMetricFamilyCollector for DiskstatsMetricCollector<T>
where
    T: TimeMeasure + Clone,
{
    fn family_name(&self) -> &'static str {
        DISKSTATS_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_diskstats_metrics()
    }
}

#[cfg(test)]
mod test {

    use std::time::Duration;

    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;

    use super::*;
    use crate::test_utils::TestInstant;

    #[rstest]
    #[case("   8       0 sda 57839 46346 3776180 24107 2868024 458991 39327918 1458693 0 839132 1568407 285564 2 308337155 49208 141738 36397", "sda", "basic_diskstats_line")]
    #[case(
        "253       1 vda1 59 125 775 15 2 0 2 0 0 64 16 0 0 0 0 0 0",
        "vda1",
        "from_qemu"
    )]
    fn test_process_valid_proc_stat_line(
        #[case] proc_diskstats_line: &str,
        #[case] expected_device_name: &str,
        #[case] test_name: &str,
    ) {
        let (device_name, disk_stats) =
            DiskstatsMetricCollector::<TestInstant>::parse_proc_diskstats_line(proc_diskstats_line)
                .unwrap();
        assert_eq!(device_name, expected_device_name);
        assert_json_snapshot!(test_name, disk_stats);
    }

    #[rstest]
    #[case("   8       0 sda 57839 46346 3776180 24107 2868024 458991 39327918 1458693 0 839132 1568407 285564 2 308337155 49208 141738 36397",  
           "   8       0 sda 57939 46346 3776180 24107 2868224 458991 39327918 1458693 0 839132 1568407 285564 2 308337155 49208 141738 36397",
           "basic_diskstats_calc")]
    fn test_calculate_metrics(
        #[case] proc_diskstats_line_a: &str,
        #[case] proc_diskstats_line_b: &str,
        #[case] test_name: &str,
    ) {
        let mut collector =
            DiskstatsMetricCollector::<TestInstant>::new(DiskstatsMetricsConfig::Auto);

        let (device_name, disk_stats) =
            DiskstatsMetricCollector::<TestInstant>::parse_proc_diskstats_line(
                proc_diskstats_line_a,
            )
            .unwrap();

        assert_eq!(device_name, "sda");

        let result_a = collector
            .delta_since_last_reading(device_name, disk_stats)
            .unwrap();

        assert!(result_a.is_none());

        TestInstant::sleep(Duration::from_secs(10));

        let (device_name, disk_stats) =
            DiskstatsMetricCollector::<TestInstant>::parse_proc_diskstats_line(
                proc_diskstats_line_b,
            )
            .unwrap();

        assert_eq!(device_name, "sda");

        let result_b = collector
            .delta_since_last_reading(device_name, disk_stats)
            .unwrap()
            .unwrap();
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(test_name,
                                  result_b,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }
    #[rstest]
    #[case("   8       0 sda 57839 46346 3776180 24107 2868024 458991 39327918 1458693 0 839132 1568407 285564 2 308337155 49208 141738 36397")]
    fn test_unmonitored_disk_ignored(#[case] proc_diskstats_line_a: &str) {
        let collector = DiskstatsMetricCollector::<TestInstant>::new(
            DiskstatsMetricsConfig::Devices(HashSet::from(["vda".to_string()])),
        );

        let (device_name, _disk_stats) =
            DiskstatsMetricCollector::<TestInstant>::parse_proc_diskstats_line(
                proc_diskstats_line_a,
            )
            .unwrap();

        assert_eq!(device_name, "sda");
        assert!(!collector.device_is_monitored(&device_name));
    }
}
