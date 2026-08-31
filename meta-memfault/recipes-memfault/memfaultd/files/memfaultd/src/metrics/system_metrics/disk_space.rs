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
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
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
pub trait DiskSpaceInfoForPath: Send {
    fn disk_space_info_for_path(p: &Path) -> Result<DiskSpaceInfo>;
}

pub struct NixStatvfs {}

impl DiskSpaceInfoForPath for NixStatvfs {
    fn disk_space_info_for_path(p: &Path) -> Result<DiskSpaceInfo> {
        let statfs = statvfs(p)
            .map_err(|e| eyre!("Failed to get statfs info for {}: {}", p.display(), e))?;

        Ok(DiskSpaceInfo {
            block_size: statfs.block_size() as _,
            blocks: statfs.blocks() as _,
            blocks_free: statfs.blocks_free() as _,
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
    mounts: Vec<Arc<Mount>>,
    _marker: PhantomData<T>,
}

impl<T> DiskSpaceMetricCollector<T>
where
    T: DiskSpaceInfoForPath,
{
    pub fn new(config: DiskSpaceMetricsConfig) -> Self {
        Self {
            config,
            mounts: Vec::new(),
            _marker: PhantomData,
        }
    }
    fn disk_is_monitored(&self, disk: &str) -> bool {
        match &self.config {
            DiskSpaceMetricsConfig::Auto => {
                disk.starts_with("/dev") && !(disk.contains("loop") || disk.contains("ram"))
            }
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

        for line in reader.lines().map_while(Result::ok) {
            // Discard errors - the assumption here is that we are only parsing
            // lines that follow the specified format and expect other lines in the file to error
            if let Ok(mount) = Self::parse_proc_mounts_line(line.trim()) {
                if self.disk_is_monitored(&mount.device.to_string_lossy()) {
                    self.mounts.push(Arc::new(mount));
                }
            }
        }
        Ok(())
    }

    /// Reads stats for every mount, keeping per-mount results separate so that
    /// one unreadable mount doesn't discard the readings for the others
    fn get_mount_stats(mounts: Vec<Arc<Mount>>) -> Vec<(Result<DiskSpaceInfo>, Arc<Mount>)> {
        mounts
            .into_iter()
            .map(|mount| {
                let mount_stats = T::disk_space_info_for_path(mount.mount_point.as_path());
                (mount_stats, mount)
            })
            .collect()
    }

    /// For a given mounted device, construct metric readings
    /// for how many bytes are used and free on the device
    /// Also takes 2 pointers used to track how much space
    /// is used in total on the system
    fn build_metrics_for_mount(
        &self,
        mount: &Mount,
        mount_stats: DiskSpaceInfo,
    ) -> Result<Vec<KeyedMetricReading>> {
        let block_size = mount_stats.block_size;
        let bytes_total = (mount_stats.blocks * block_size) as f64;
        let bytes_free = (mount_stats.blocks_free * block_size) as f64;
        let bytes_used = bytes_total - bytes_free;

        let disk_id = mount
            .device
            .file_name()
            .ok_or_else(|| eyre!("Couldn't extract basename"))?
            .to_string_lossy();

        if bytes_total > 0.0 {
            let bytes_free_reading = KeyedMetricReading::new_histogram(
                format!("disk_space/{}/free_bytes", disk_id)
                    .as_str()
                    .parse()
                    .map_err(|e| eyre!("Couldn't parse metric key for bytes free: {}", e))?,
                bytes_free,
            );

            let bytes_used_reading = KeyedMetricReading::new_histogram(
                format!("disk_space/{}/used_bytes", disk_id)
                    .as_str()
                    .parse()
                    .map_err(|e| eyre!("Couldn't parse metric key for bytes used: {}", e))?,
                bytes_used,
            );

            let _storage_disk_pct = (bytes_used / bytes_total) * 100.0;

            Ok(vec![bytes_free_reading, bytes_used_reading])
        } else {
            Err(eyre!(
                "Total bytes for {} is not a positive number ({}) - can't calculate metrics.",
                disk_id,
                bytes_total,
            ))
        }
    }

    pub fn get_disk_space_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        if self.mounts.is_empty() {
            self.initialize_mounts(Path::new(PROC_MOUNTS_PATH))?;
        }

        let mut disk_space_readings = Vec::new();
        let mounts = self.mounts.clone();
        let mount_stats = Self::get_mount_stats(mounts);
        for (mount_stats, mount) in mount_stats {
            match mount_stats.and_then(|stats| self.build_metrics_for_mount(&mount, stats)) {
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
    T: DiskSpaceInfoForPath + Send,
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
    use std::sync::Mutex;
    use tempfile::{tempdir, TempDir};

    use super::*;

    type TestCollector = DiskSpaceMetricCollector<MockDiskSpaceInfoForPath>;

    // Mockall keeps expectations for associated functions in a global, so tests
    // that set them have to be serialized against each other.
    static STATVFS_MTX: Mutex<()> = Mutex::new(());

    const MOUNTS_LINES: [&str; 2] = [
        "/dev/sda2 /media ext4 rw,noatime 0 0",
        "/dev/sda1 / ext4 rw,noatime 0 0",
    ];

    fn write_mounts_file(dir: &TempDir, lines: &[&str]) -> PathBuf {
        let mounts_file_path = dir.path().join("mounts");
        let mut mounts_file = File::create(&mounts_file_path).unwrap();
        for line in lines {
            writeln!(mounts_file, "{}", line).unwrap();
        }
        mounts_file_path
    }

    fn mount(device: &str, mount_point: &str) -> Mount {
        Mount {
            device: Path::new(device).to_path_buf(),
            mount_point: Path::new(mount_point).to_path_buf(),
        }
    }

    fn disk_space_info(block_size: u64, blocks: u64, blocks_free: u64) -> DiskSpaceInfo {
        DiskSpaceInfo {
            block_size,
            blocks,
            blocks_free,
        }
    }

    fn sorted_mounts(collector: &mut TestCollector) -> Vec<&Mount> {
        collector.mounts.sort_by(|a, b| a.device.cmp(&b.device));
        collector.mounts.iter().map(Arc::as_ref).collect()
    }

    #[rstest]
    fn test_process_valid_proc_mounts_line() {
        let line = "/dev/sda2 /media ext4 rw,noatime 0 0";
        let mount = TestCollector::parse_proc_mounts_line(line).unwrap();

        assert_eq!(mount.device.as_os_str().to_string_lossy(), "/dev/sda2");
        assert_eq!(mount.mount_point.as_os_str().to_string_lossy(), "/media");
    }

    #[rstest]
    fn test_initialize_mounts() {
        let mut disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        let dir = tempdir().unwrap();
        let mounts_file_path = write_mounts_file(&dir, &MOUNTS_LINES);

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        assert_json_snapshot!(sorted_mounts(&mut disk_space_collector));

        dir.close().unwrap();
    }

    #[rstest]
    fn test_initialize_mounts_skips_unparsable_lines() {
        let mut disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        let dir = tempdir().unwrap();
        let mounts_file_path = write_mounts_file(
            &dir,
            &[
                "",
                "garbage",
                MOUNTS_LINES[0],
                "/dev/sda3-with-no-mount-point",
            ],
        );

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        let mounts = sorted_mounts(&mut disk_space_collector);
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].device, Path::new("/dev/sda2"));

        dir.close().unwrap();
    }

    #[rstest]
    fn test_initialize_mounts_missing_file() {
        let mut disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        assert!(disk_space_collector
            .initialize_mounts(Path::new("/nonexistent/proc/mounts"))
            .is_err());
    }

    #[rstest]
    fn test_unmonitored_disks_not_initialized() {
        let mut disk_space_collector =
            TestCollector::new(DiskSpaceMetricsConfig::Disks(HashSet::from_iter([
                "/dev/sdc1".to_string(),
            ])));

        let dir = tempdir().unwrap();
        let mounts_file_path = write_mounts_file(&dir, &MOUNTS_LINES);

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        assert!(disk_space_collector.mounts.is_empty());

        dir.close().unwrap();
    }

    #[rstest]
    fn test_get_mount_stats() {
        let _guard = STATVFS_MTX.lock().unwrap();
        let statvfs_ctx = MockDiskSpaceInfoForPath::disk_space_info_for_path_context();
        statvfs_ctx
            .expect()
            .times(2)
            .returning(|p| match p.to_string_lossy().as_ref() {
                "/media" => Ok(disk_space_info(4096, 1024, 286)),
                _ => Ok(disk_space_info(512, 2048, 512)),
            });

        let mounts = vec![
            Arc::new(mount("/dev/sda2", "/media")),
            Arc::new(mount("/dev/sda1", "/")),
        ];

        let mount_stats = TestCollector::get_mount_stats(mounts);

        // Each DiskSpaceInfo must stay paired with the mount it was read for
        assert_eq!(mount_stats.len(), 2);
        assert_eq!(mount_stats[0].1.mount_point, Path::new("/media"));
        assert_eq!(mount_stats[0].0.as_ref().unwrap().blocks_free, 286);
        assert_eq!(mount_stats[1].1.mount_point, Path::new("/"));
        assert_eq!(mount_stats[1].0.as_ref().unwrap().blocks_free, 512);
    }

    #[rstest]
    fn test_get_mount_stats_error_isolated_to_failing_mount() {
        let _guard = STATVFS_MTX.lock().unwrap();
        let statvfs_ctx = MockDiskSpaceInfoForPath::disk_space_info_for_path_context();
        statvfs_ctx
            .expect()
            .times(2)
            .returning(|p| match p.to_string_lossy().as_ref() {
                "/media" => Err(eyre!("statvfs failed")),
                _ => Ok(disk_space_info(512, 2048, 512)),
            });

        let mounts = vec![
            Arc::new(mount("/dev/sda2", "/media")),
            Arc::new(mount("/dev/sda1", "/")),
        ];

        let mount_stats = TestCollector::get_mount_stats(mounts);

        assert_eq!(mount_stats.len(), 2);
        assert!(mount_stats[0].0.is_err());
        assert_eq!(mount_stats[1].0.as_ref().unwrap().blocks_free, 512);
    }

    #[rstest]
    fn test_collect_metrics_skips_failing_mount() {
        let _guard = STATVFS_MTX.lock().unwrap();
        let statvfs_ctx = MockDiskSpaceInfoForPath::disk_space_info_for_path_context();
        statvfs_ctx
            .expect()
            .times(2)
            .returning(|p| match p.to_string_lossy().as_ref() {
                "/media" => Err(eyre!("statvfs failed")),
                _ => Ok(disk_space_info(512, 2048, 512)),
            });

        let mut disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        let dir = tempdir().unwrap();
        let mounts_file_path = write_mounts_file(&dir, &MOUNTS_LINES);

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        // The unreadable /media mount is warned about and skipped, the healthy
        // mount still reports readings
        let metrics = disk_space_collector.collect_metrics().unwrap();

        let names: Vec<_> = metrics.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["disk_space/sda1/free_bytes", "disk_space/sda1/used_bytes"]
        );

        dir.close().unwrap();
    }

    #[rstest]
    fn test_build_metrics_for_mount() {
        let disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        let metrics = disk_space_collector
            .build_metrics_for_mount(
                &mount("/dev/mmcblk0p2", "/data"),
                disk_space_info(512, 2048, 512),
            )
            .unwrap();

        assert_json_snapshot!(metrics,
            {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)}
        );
    }

    #[rstest]
    fn test_build_metrics_for_mount_zero_total_bytes() {
        let disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        assert!(disk_space_collector
            .build_metrics_for_mount(&mount("/dev/sda1", "/"), disk_space_info(4096, 0, 0))
            .is_err());
    }

    #[rstest]
    fn test_build_metrics_for_mount_device_without_basename() {
        let disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        assert!(disk_space_collector
            .build_metrics_for_mount(&mount("/", "/"), disk_space_info(512, 2048, 512))
            .is_err());
    }

    #[rstest]
    fn test_initialize_and_calc_disk_space_for_mounts() {
        let _guard = STATVFS_MTX.lock().unwrap();
        let statvfs_ctx = MockDiskSpaceInfoForPath::disk_space_info_for_path_context();
        statvfs_ctx
            .expect()
            .times(2)
            .returning(|_p| Ok(disk_space_info(4096, 1024, 286)));

        let mut disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        let dir = tempdir().unwrap();
        let mounts_file_path = write_mounts_file(&dir, &MOUNTS_LINES);

        assert!(disk_space_collector
            .initialize_mounts(&mounts_file_path)
            .is_ok());

        let mut metrics = disk_space_collector.collect_metrics().unwrap();
        // collect_metrics preserves /proc/mounts order, which read order makes
        // unstable across runs
        metrics.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));

        assert_json_snapshot!(metrics,
                                  {"[].value.**.timestamp" => "[timestamp]", "[].value.**.value" => rounded_redaction(5)}
        );

        dir.close().unwrap();
    }

    #[rstest]
    #[case("/dev/sda2", true)]
    #[case("/dev/loop0", false)]
    #[case("/dev/ram0", false)]
    fn test_disk_monitored(#[case] disk: &str, #[case] expected: bool) {
        let disk_space_collector = TestCollector::new(DiskSpaceMetricsConfig::Auto);

        assert_eq!(disk_space_collector.disk_is_monitored(disk), expected);
    }
}
