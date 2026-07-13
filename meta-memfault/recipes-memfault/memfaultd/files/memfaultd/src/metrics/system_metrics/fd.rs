//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect file descriptor usage system stats from `/proc/sys/fs/file-nr`
//!
//! This module parses FD statistics and constructs `KeyedMetricReadings`
//! based on those statistics.
//!
//! Example `/proc/sys/fs/file-nr` contents:
//! `7759    0       9223372036854775807`
//!
//! In order, these are allocated file handles, free file handles,
//! and max file handles on a system.
//! Note that since Linux 2.6, free file handles is always zero.
//! As a result we only emit `fs/file_nr/allocated` and `fs/file_nr/max`
//! corresponding to the first and third numbers listed above.
//!
//! When changing these customer-facing metric keys, also update
//! https://github.com/memfault/memfault-docs so users can discover and
//! configure the metrics.
//!
//! You can see the man pages for sys fs here:
//! https://man7.org/linux/man-pages/man5/proc_sys_fs.5.html
//! Or docs from the Linux kernel docs here:
//! https://docs.kernel.org/admin-guide/sysctl/fs.html
use nom::{
    character::complete::{char, space1, u64},
    sequence::{preceded, separated_pair},
    IResult,
};

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

#[cfg(test)]
use std::iter::zip;

use eyre::{eyre, Result};

use crate::metrics::{
    system_metrics::SystemMetricFamilyCollector, KeyedMetricReading, MetricStringKey,
};

#[cfg(test)]
use crate::metrics::metric_reading::MetricReading;

const FILE_NR_PATH: &str = "/proc/sys/fs/file-nr";
pub const FD_METRIC_NAMESPACE: &str = "fs/handles";

pub struct FdMetricCollector;

impl FdMetricCollector {
    pub fn new() -> Self {
        Self
    }

    fn calculate_fd_metrics(&mut self, line: &str) -> Result<Vec<KeyedMetricReading>> {
        let (alloc_info, max_info) = Self::parse_fdmetric(line)?;

        Ok(vec![
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("fs/handles/allocated"),
                alloc_info as f64,
            ),
            KeyedMetricReading::new_gauge(MetricStringKey::from("fs/handles/max"), max_info as f64),
        ])
    }

    pub fn get_fd_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let path = Path::new(FILE_NR_PATH);
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let mut buf = String::new();
        reader.read_to_string(&mut buf)?;

        self.calculate_fd_metrics(&buf)
    }

    /// Given a pattern of
    /// `X    0       Z`
    /// Returns the values represented by `X` and `Z`.
    /// As of Linux 2.6, `/proc/sys/fs/file-nr` effectively returns a constant `0`
    /// for the "free file handles" field
    fn parse_fd_stats(input: &str) -> IResult<&str, (u64, u64)> {
        separated_pair(u64, preceded(space1, char('0')), preceded(space1, u64))(input)
    }

    /// Wrapper around `parse_fd_stats` for better error messaging
    fn parse_fdmetric(line: &str) -> Result<(u64, u64)> {
        let (_, (alloc_info, max_info)) = Self::parse_fd_stats(line)
            .map_err(|_e| eyre!("Failed to parse FD stats line: {}", line))?;
        Ok((alloc_info, max_info))
    }
}

impl SystemMetricFamilyCollector for FdMetricCollector {
    fn family_name(&self) -> &'static str {
        FD_METRIC_NAMESPACE
    }
    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_fd_metrics()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("7759   0      9223372036854775807", (7759, 9223372036854775807))]
    #[case("1 0 2", (1, 2))]
    #[case("92834570876 0 0", (92834570876, 0))]
    #[case("1                                                                                          0 2", (1, 2))]
    #[case("1 0                                                                                          2", (1, 2))]
    fn test_valid_filenr_line(#[case] filenr_line: &str, #[case] expected: (u64, u64)) {
        let parsed =
            FdMetricCollector::parse_fdmetric(filenr_line).expect("fed valid filenr fails parsing");
        assert_eq!(parsed, expected);
    }

    #[rstest]
    #[case("1 0  ")]
    #[case("92834570876 00")]
    #[case("1 2 3 4")]
    #[case("1                                                                                           2")]
    fn test_invalid_filenr_line_is_err(#[case] filenr_line: &str) {
        let parsed = FdMetricCollector::parse_fdmetric(filenr_line);
        assert!(parsed.is_err());
    }

    #[rstest]
    #[case("7759   0      922",
        Ok(vec![
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("fs/file_nr/allocated"),
                7759f64,
            ),
            KeyedMetricReading::new_gauge(MetricStringKey::from("fs/file_nr/max"), 922f64),
        ])
    )]
    #[case("1 0 2",
        Ok(vec![
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("fs/file_nr/allocated"),
                1f64,
            ),
            KeyedMetricReading::new_gauge(MetricStringKey::from("fs/file_nr/max"), 2f64),
        ])
    )]
    #[case("928 0 0",
        Ok(vec![
            KeyedMetricReading::new_gauge(
                MetricStringKey::from("fs/file_nr/allocated"),
                928f64,
            ),
            KeyedMetricReading::new_gauge(MetricStringKey::from("fs/file_nr/max"), 0f64),
        ])
    )]
    fn test_calculate_fd_metrics(
        #[case] filenr_line: &str,
        #[case] expected: Result<Vec<KeyedMetricReading>>,
    ) {
        let res = FdMetricCollector::new().calculate_fd_metrics(filenr_line);
        match expected {
            Err(_) => assert!(res.is_err()),
            Ok(vec_expected) => {
                let vec_res = res.expect("result expected to also be ok");
                zip(vec_expected, vec_res).for_each(|(kmr_expected, kmr_res)| {
                    let mr_expected = kmr_expected.value;
                    let mr_res = kmr_res.value;

                    match mr_expected {
                        MetricReading::Gauge {
                            value: val_expected,
                            ..
                        } => match mr_res {
                            MetricReading::Gauge { value: val_res, .. } => {
                                assert_eq!(val_expected, val_res)
                            }
                            _ => panic!(
                                "expected MetricReading and result MetricReading differ variant"
                            ),
                        },
                        _ => panic!("expected a MetricReading::Gauge, got a {:#?}", mr_expected),
                    }
                })
            }
        }
    }
}
