//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect disk space metric readings for devices listed in
//! /proc/mounts
//!
//! This module parses mounted devices and their mount points
//! from /proc/mounts and calculates how many bytes are free
//! and used on the device.
//!
use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use eyre::{eyre, Result};
use log::warn;
use nix::sys::statvfs::statvfs;
use nom::{
    bytes::complete::take_while,
    character::complete::multispace1,
    sequence::{pair, preceded},
    IResult,
};

use serde::Serialize;

use crate::metrics::{system_metrics::SystemMetricFamilyCollector, KeyedMetricReading};

pub const DISKSPACE_METRIC_NAMESPACE_LEGACY: &str = "df";
pub const DISKSPACE_METRIC_NAMESPACE: &str = "disk_space";
pub const PROC_MOUNTS_PATH: &str = "/proc/mounts";

pub struct DiskSpaceInfo {
    block_size: u64,
    blocks: u64,
    blocks_free: u64,
}

#[cfg_attr(test, mockall::automock)]
pub trait DiskSpaceInfoForPath {
    fn disk_space_info_for_path(&self, p: &Path) -> Result<DiskSpaceInfo>;
}

pub struct NixStatvfs {}

impl NixStatvfs {
    pub fn new() -> Self {
        Self {}
    }
}

impl DiskSpaceInfoForPath for NixStatvfs {
    fn disk_space_info_for_path(&self, p: &Path) -> Result<DiskSpaceInfo> {
        let statfs = statvfs(p)
            .map_err(|e| eyre!("Failed to get statfs info for {}: {}", p.display(), e))?;

        Ok(DiskSpaceInfo {
            // Ignore unnecessary cast for these
            // as it is needed on 32-bit systems.
            #[allow(clippy::unnecessary_cast)]
            block_size: statfs.block_size() as u64,
            #[allow(clippy::unnecessary_cast)]
            blocks: statfs.blocks() as u64,
            #[allow(clippy::unnecessary_cast)]
            blocks_free: statfs.blocks_free() as u64,
        })
    }
}

#[derive(Serialize)]
struct Mount {
    device: PathBuf,
    mount_point: PathBuf,
}

pub enum DiskSpaceMetricsConfig {
    Auto,
    Disks(HashSet<String>),
}

pub struct DiskSpaceMetricCollector<T>
where
    T: DiskSpaceInfoForPath,
{
    config: DiskSpaceMetricsConfig,
    mounts: Vec<Mount>,
    disk_space_impl: T,
}

impl<T> DiskSpaceMetricCollector<T>
where
    T: DiskSpaceInfoForPath,
{
    pub fn new(disk_space_impl: T, config: DiskSpaceMetricsConfig) -> Self {
        Self {
            config,
            mounts: Vec::new(),
            disk_space_impl,
        }
    }
    fn disk_is_monitored(&self, disk: &str) -> bool {
        match &self.config {
            DiskSpaceMetricsConfig::Auto => disk.starts_with("/dev"),
            DiskSpaceMetricsConfig::Disks(configured_disks) => configured_disks.contains(disk),
        }
    }

    /// Parses a line of /proc/mounts for the name of
    /// the device the line corresponds to
    ///
    /// Example input:
    /// "/dev/sda2 / ext4 rw,noatime 0 0"
    /// Example output:
    /// "/dev/sda2"
    fn parse_proc_mounts_device(proc_mounts_line: &str) -> IResult<&str, &str> {
        take_while(|c: char| !c.is_whitespace())(proc_mounts_line)
    }

    /// Parses a line of /proc/mounts for the
    /// mount point the line corresponds to
    /// Parses /proc/mounts for a list of devices with active
    /// mount points in the system
    /// Example input:
    /// " / ext4 rw,noatime 0 0"
    /// Example output:
    /// "/"
    fn parse_proc_mounts_mount_point(proc_mounts_line: &str) -> IResult<&str, &str> {
        preceded(multispace1, take_while(|c: char| !c.is_whitespace()))(proc_mounts_line)
    }

    /// Parse a line of /proc/mounts
    /// Example input:
    /// "/dev/sda2 / ext4 rw,noatime 0 0"
    /// Example output:
    /// Mount { device: "/dev/sda2", "mount_point": "/" }
    fn parse_proc_mounts_line(line: &str) -> Result<Mount> {
        let (_remaining, (device, mount_point)) = pair(
            Self::parse_proc_mounts_device,
            Self::parse_proc_mounts_mount_point,
        )(line)
        .map_err(|e| eyre!("Failed to parse /proc/mounts line: {}", e))?;
        Ok(Mount {
            device: Path::new(device).to_path_buf(),
            mount_point: Path::new(mount_point).to_path_buf(),
        })
    }

    /// Initialize the list of mounted devices and their mount points based
    /// on the contents of /proc/mounts
    pub fn initialize_mounts(&mut self, proc_mounts_path: &Path) -> Result<()> {
        let file = File::open(proc_mounts_path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            // Discard errors - the assumption here is that we are only parsing
            // lines that follow the specified format and expect other lines in the file to error
            if let Ok(mount) = Self::parse_proc_mounts_line(line?.trim()) {
                if self.disk_is_monitored(&mount.device.to_string_lossy()) {
                    self.mounts.push(mount);
                }
            }
        }
        Ok(())
    }

    /// For a given mounted device, construct metric readings
    /// for how many bytes are used and free on the device
    fn get_metrics_for_mount(&self, mount: &Mount) -> Result<Vec<KeyedMetricReading>> {
        let mount_stats = self
            .disk_space_impl
            .disk_space_info_for_path(mount.mount_point.as_path())?;

        let block_size = mount_stats.block_size;
        let bytes_free = mount_stats.blocks_free * block_size;
        let bytes_used = (mount_stats.blocks * block_size) - bytes_free;

        let disk_id = mount
            .device
            .file_name()
            .ok_or_else(|| eyre!("Couldn't extract basename"))?
            .to_string_lossy();

        let bytes_free_reading = KeyedMetricReading::new_histogram(
            format!("disk_space/{}/free_bytes", disk_id)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key for bytes free: {}", e))?,
            bytes_free as f64,
        );

        let bytes_used_reading = KeyedMetricReading::new_histogram(
            format!("disk_space/{}/used_bytes", disk_id)
                .as_str()
                .parse()
                .map_err(|e| eyre!("Couldn't parse metric key for bytes used: {}", e))?,
            bytes_used as f64,
        );

        Ok(vec![bytes_free_reading, bytes_used_reading])
    }

    pub fn get_disk_space_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        if self.mounts.is_empty() {
            self.initialize_mounts(Path::new(PROC_MOUNTS_PATH))?;
        }

        let mut disk_space_readings = Vec::new();
        for mount in self.mounts.iter() {
            match self.get_metrics_for_mount(mount) {
                Ok(readings) => disk_space_readings.extend(readings),
                Err(e) => warn!(
                    "Failed to calculate disk space readings for {} mounted at {}: {}",
                    mount.device.display(),
                    mount.mount_point.display(),
                    e
                ),
            }
        }

        Ok(disk_space_readings)
    }
}

impl<T> SystemMetricFamilyCollector for DiskSpaceMetricCollector<T>
where
    T: DiskSpaceInfoForPath,
{
    fn family_name(&self) -> &'static str {
        DISKSPACE_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_disk_space_metrics()
    }
}

#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::Write;

    use insta::{assert_json_snapshot, rounded_redaction};
    use rstest::rstest;
    use tempfile::tempdir;

    use super::*;

    #[rstest]
    fn test_process_valid_proc_mounts_line() {
        let line = "/dev/sda2 /media ext4 rw,noatime 0 0";
        let mount =
            DiskSpaceMetricCollector::<MockDiskSpaceInfoForPath>::parse_proc_mounts_line(line)
                .unwrap();

        assert_eq!(mount.device.as_os_str().to_string_lossy(), "/dev/sda2");
        assert_eq!(mount.mount_point.as_os_str().to_string_lossy(), "/media");
    }

    #[rstest]
    fn test_initialize_and_calc_disk_space_for_mounts() {
        let mut mock_statfs = MockDiskSpaceInfoForPath::new();

        mock_statfs
            .expect_disk_space_info_for_path()
            .times(2)
            .returning(|_p| {
                Ok(DiskSpaceInfo {
                    block_size: 4096,
                    blocks: 1024,
                    blocks_free: 286,
                })
            });

        let mut disk_space_collector =
            DiskSpaceMetricCollector::new(mock_statfs, DiskSpaceMetricsConfig::Auto);

        let line = "/dev/sda2 /media ext4 rw,noatime 0 0";
        let line2 = "/dev/sda1 / ext4 rw,noatime 0 0";

        let dir = tempdir().unwrap();

        let mounts_file_path = dir.path().join("mounts");
        let mut mounts_file = File::create(mounts_file_path.clone()).unwrap();

        writeln!(mounts_file, "{}", line).unwrap();
        writeln!(mounts_file, "{}", line2).unwrap();

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        disk_space_collector
            .mounts
            .sort_by(|a, b| a.device.cmp(&b.device));

        assert_json_snapshot!(disk_space_collector.mounts);

        let metrics = disk_space_collector.collect_metrics().unwrap();

        assert_json_snapshot!(metrics,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)}
        );

        dir.close().unwrap();
    }
    #[rstest]
    fn test_unmonitored_disks_not_initialized() {
        let mock_statfs = MockDiskSpaceInfoForPath::new();

        let mut disk_space_collector = DiskSpaceMetricCollector::new(
            mock_statfs,
            DiskSpaceMetricsConfig::Disks(HashSet::from_iter(["/dev/sdc1".to_string()])),
        );

        let line = "/dev/sda2 /media ext4 rw,noatime 0 0";
        let line2 = "/dev/sda1 / ext4 rw,noatime 0 0";

        let dir = tempdir().unwrap();

        let mounts_file_path = dir.path().join("mounts");
        let mut mounts_file = File::create(mounts_file_path.clone()).unwrap();

        writeln!(mounts_file, "{}", line).unwrap();
        writeln!(mounts_file, "{}", line2).unwrap();

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        disk_space_collector
            .mounts
            .sort_by(|a, b| a.device.cmp(&b.device));

        assert!(disk_space_collector.mounts.is_empty());

        dir.close().unwrap();
    }
}
