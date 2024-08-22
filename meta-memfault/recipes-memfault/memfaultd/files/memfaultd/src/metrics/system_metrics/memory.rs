//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect memory metric readings from /proc/meminfo
//!
//! This module parses memory statistics from /proc/meminfo and
//! constructs KeyedMetricReadings based on those statistics.
//!
//! Example /proc/meminfo contents:
//! MemTotal:         365916 kB
//! MemFree:          242276 kB
//! MemAvailable:     292088 kB
//! Buffers:            4544 kB
//! Cached:            52128 kB
//! SwapCached:            0 kB
//! Active:            21668 kB
//! Inactive:          51404 kB
//! Active(anon):       2312 kB
//! Inactive(anon):    25364 kB
//! Active(file):      19356 kB
//! Inactive(file):    26040 kB
//! Unevictable:        3072 kB
//! Mlocked:               0 kB
//! SwapTotal:             0 kB
//! SwapFree:              0 kB
//! Dirty:                 0 kB
//! Writeback:             0 kB
//! AnonPages:         19488 kB
//! Mapped:            29668 kB
//! Shmem:             11264 kB
//! KReclaimable:      14028 kB
//! Slab:              32636 kB
//! SReclaimable:      14028 kB
//!
//! Only the following lines are currently processed:
//! MemTotal, MemFree, and optionally MemAvailable
//!
//! These lines are used by this module to calculate
//! free and used memory. MemFree is used in place of
//! MemAvailable if the latter is not present.
//!
//!
//! See additional Linux kernel documentation on /proc/meminfo here:
//! https://www.kernel.org/doc/html/latest/filesystems/proc.html#meminfo
use std::fs::read_to_string;
use std::path::Path;
use std::{collections::HashMap, str::FromStr};

use eyre::{eyre, Result};
use nom::{
    bytes::complete::tag,
    character::complete::{alpha1, multispace1},
    number::complete::double,
    sequence::{delimited, terminated},
    IResult,
};

use crate::metrics::{
    core_metrics::METRIC_MEMORY_PCT, system_metrics::SystemMetricFamilyCollector,
    KeyedMetricReading, MetricStringKey,
};

const PROC_MEMINFO_PATH: &str = "/proc/meminfo";
pub const MEMORY_METRIC_NAMESPACE: &str = "memory";

pub struct MemoryMetricsCollector;

impl MemoryMetricsCollector {
    pub fn new() -> Self {
        MemoryMetricsCollector {}
    }

    pub fn get_memory_metrics(&self) -> Result<Vec<KeyedMetricReading>> {
        let path = Path::new(PROC_MEMINFO_PATH);
        // Need to read all of /proc/meminfo at once
        // as we derive used memory based on a calculation
        // using multiple lines
        let contents = read_to_string(path)?;
        Self::parse_meminfo_memory_stats(&contents)
    }

    /// Parses the key in a /proc/meminfo line
    ///
    /// A key is the string terminated by the `:` character
    /// In the following line, "MemTotal" would be parsed as the key
    /// MemTotal:         365916 kB
    fn parse_meminfo_key(meminfo_line: &str) -> IResult<&str, &str> {
        terminated(alpha1, tag(":"))(meminfo_line)
    }

    /// Parses the kilobyte value in a /proc/meminfo line
    ///
    /// This value is the kilobytes used by the corresponding
    /// meminfo key. The value is a number terminated by " kB".
    /// In the following line, 365916.0 would be parsed by
    /// this function as the kB used by "MemTotal"
    /// MemTotal:         365916 kB
    fn parse_meminfo_kb(meminfo_line_suffix: &str) -> IResult<&str, f64> {
        delimited(multispace1, double, tag(" kB"))(meminfo_line_suffix)
    }

    /// Parses a full /proc/meminfo contents and returns
    /// a vector of KeyedMetricReadings
    fn parse_meminfo_memory_stats(meminfo: &str) -> Result<Vec<KeyedMetricReading>> {
        let mut stats = meminfo
            .trim()
            .lines()
            .map(|line| -> Result<(&str, f64), String> {
                let (suffix, key) = Self::parse_meminfo_key(line).map_err(|e| e.to_string())?;
                let (_, kb) = Self::parse_meminfo_kb(suffix).map_err(|e| e.to_string())?;
                // Use bytes as unit instead of KB
                Ok((key, kb * 1024.0))
            })
            .filter_map(|result| result.ok())
            .collect::<HashMap<&str, f64>>();

        // Use the same methodology as `free` to calculate used memory.
        //
        // For kernels 3.14 and greater:
        // MemUsed = MemTotal - MemAvailable
        //
        // For kernels less than 3.14 (no MemAvailable):
        // MemUsed = MemTotal - MemFree
        //
        // See below man page for more info:
        // https://man7.org/linux/man-pages/man1/free.1.html
        let total = stats
            .remove("MemTotal")
            .ok_or_else(|| eyre!("{} is missing required value MemTotal", PROC_MEMINFO_PATH))?;
        let free = stats
            .remove("MemFree")
            .ok_or_else(|| eyre!("{} is missing required value MemFree", PROC_MEMINFO_PATH))?;

        // Check that MemTotal is nonzero to avoid dividing by 0
        if total != 0.0 {
            let available = stats.remove("MemAvailable").unwrap_or(free);

            let used = total - available;

            let pct_used = (used / total) * 100.0;

            let used_key = MetricStringKey::from_str("memory/memory/used")
                .map_err(|e| eyre!("Failed to construct MetricStringKey for used memory: {}", e))?;
            let free_key = MetricStringKey::from_str("memory/memory/free")
                .map_err(|e| eyre!("Failed to construct MetricStringKey for used memory: {}", e))?;

            let pct_key = MetricStringKey::from_str(METRIC_MEMORY_PCT)
                .map_err(|e| eyre!("Failed to construct MetricStringKey for used memory: {}", e))?;

            Ok(vec![
                KeyedMetricReading::new_histogram(free_key, free),
                KeyedMetricReading::new_histogram(used_key, used),
                KeyedMetricReading::new_histogram(pct_key, pct_used),
            ])
        } else {
            Err(eyre!("MemTotal is 0, can't calculate memory usage metrics"))
        }
    }
}

impl SystemMetricFamilyCollector for MemoryMetricsCollector {
    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_memory_metrics()
    }

    fn family_name(&self) -> &'static str {
        MEMORY_METRIC_NAMESPACE
    }
}

#[cfg(test)]
mod test {

    use insta::{assert_json_snapshot, rounded_redaction, with_settings};
    use rstest::rstest;

    use super::MemoryMetricsCollector;

    #[rstest]
    #[case("MemTotal:         365916 kB", "MemTotal", 365916.0)]
    #[case("MemFree:          242276 kB", "MemFree", 242276.0)]
    #[case("MemAvailable:     292088 kB", "MemAvailable", 292088.0)]
    #[case("Buffers:            4544 kB", "Buffers", 4544.0)]
    #[case("Cached:            52128 kB", "Cached", 52128.0)]
    fn test_parse_meminfo_line(
        #[case] proc_meminfo_line: &str,
        #[case] expected_key: &str,
        #[case] expected_value: f64,
    ) {
        let (suffix, key) = MemoryMetricsCollector::parse_meminfo_key(proc_meminfo_line).unwrap();
        let (_, kb) = MemoryMetricsCollector::parse_meminfo_kb(suffix).unwrap();

        assert_eq!(key, expected_key);
        assert_eq!(kb, expected_value);
    }

    #[rstest]
    fn test_get_memory_metrics() {
        let meminfo = "MemTotal:         365916 kB
MemFree:          242276 kB
MemAvailable:     292088 kB
Buffers:            4544 kB
Cached:            52128 kB
SwapCached:            0 kB
Active:            21668 kB
Inactive:          51404 kB
Active(anon):       2312 kB
Inactive(anon):    25364 kB
Active(file):      19356 kB
Inactive(file):    26040 kB
Unevictable:        3072 kB
Mlocked:               0 kB
SwapTotal:             0 kB
SwapFree:              0 kB
Dirty:                 0 kB
Writeback:             0 kB
AnonPages:         19488 kB
Mapped:            29668 kB
Shmem:             11264 kB
KReclaimable:      14028 kB
        ";

        with_settings!({sort_maps => true}, {
        assert_json_snapshot!(
                              MemoryMetricsCollector::parse_meminfo_memory_stats(meminfo).unwrap(),
                              {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }

    #[rstest]
    fn test_get_memory_metrics_no_memavailable() {
        let meminfo = "MemTotal:         365916 kB
MemFree:          242276 kB
Buffers:            4544 kB
Cached:            52128 kB
SwapCached:            0 kB
Active:            21668 kB
Inactive:          51404 kB
Active(anon):       2312 kB
Inactive(anon):    25364 kB
Active(file):      19356 kB
Inactive(file):    26040 kB
Unevictable:        3072 kB
Mlocked:               0 kB
SwapTotal:             0 kB
SwapFree:              0 kB
Dirty:                 0 kB
Writeback:             0 kB
AnonPages:         19488 kB
Mapped:            29668 kB
Shmem:             11264 kB
KReclaimable:      14028 kB
        ";

        with_settings!({sort_maps => true}, {
        assert_json_snapshot!(
                              MemoryMetricsCollector::parse_meminfo_memory_stats(meminfo).unwrap(),
                              {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)})
        });
    }

    #[rstest]
    fn test_fail_to_parse_bad_meminfo_line() {
        assert!(MemoryMetricsCollector::parse_meminfo_key("MemFree=1080kB").is_err());
        assert!(MemoryMetricsCollector::parse_meminfo_kb("1080 mB").is_err());
    }

    #[rstest]
    fn test_fail_get_metrics_with_missing_required_lines() {
        // MemFree is missing
        let meminfo = "MemTotal:         365916 kB
MemAvailable:     292088 kB
Buffers:            4544 kB
Cached:            52128 kB
SwapCached:            0 kB
Active:            21668 kB
Inactive:          51404 kB
Active(anon):       2312 kB
Inactive(anon):    25364 kB
Active(file):      19356 kB
Inactive(file):    26040 kB
Unevictable:        3072 kB
Mlocked:               0 kB
SwapTotal:             0 kB
SwapFree:              0 kB
Dirty:                 0 kB
Writeback:             0 kB
AnonPages:         19488 kB
Mapped:            29668 kB
Shmem:             11264 kB
KReclaimable:      14028 kB
        ";
        assert!(MemoryMetricsCollector::parse_meminfo_memory_stats(meminfo).is_err());
    }

    #[rstest]
    fn test_fail_get_metrics_with_bad_fmt() {
        // Not properly formatted with newlines between each key / kB pair
        let meminfo = "MemTotal:         365916 kB MemFree:          242276 kB
Buffers:            4544 kB Cached:            52128 kB Shmem:             11264 kB
        ";
        assert!(MemoryMetricsCollector::parse_meminfo_memory_stats(meminfo).is_err());
    }

    #[rstest]
    fn test_fail_get_metrics_when_mem_total_is_zero() {
        // MemFree is missing
        let meminfo = "MemTotal:         0 kB
MemAvailable:     292088 kB
Buffers:            4544 kB
Cached:            52128 kB
SwapCached:            0 kB
Active:            21668 kB
Inactive:          51404 kB
Active(anon):       2312 kB
Inactive(anon):    25364 kB
Active(file):      19356 kB
Inactive(file):    26040 kB
Unevictable:        3072 kB
Mlocked:               0 kB
SwapTotal:             0 kB
SwapFree:              0 kB
Dirty:                 0 kB
Writeback:             0 kB
AnonPages:         19488 kB
Mapped:            29668 kB
Shmem:             11264 kB
KReclaimable:      14028 kB
        ";
        assert!(MemoryMetricsCollector::parse_meminfo_memory_stats(meminfo).is_err());
    }
}
