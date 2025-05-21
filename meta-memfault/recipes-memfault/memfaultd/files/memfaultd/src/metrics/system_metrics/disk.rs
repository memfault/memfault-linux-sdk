//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashMap,
    fs::{read_dir, File},
    io::{BufRead, BufReader},
    str::FromStr,
};

use eyre::{eyre, Result};
use log::{debug, error, warn};

use crate::{
    metrics::{KeyedMetricReading, MetricStringKey},
    mmc::Mmc,
};

use super::diskstats::DiskstatsMetricsConfig;
use super::{diskstats::parse_proc_diskstats_line, SystemMetricFamilyCollector};

const PROC_DISKSTATS_PATH: &str = "/proc/diskstats";
pub const DISK_METRIC_NAMESPACE: &str = "diskstats";

const TRACKED_DISK_PREFIX: &str = "mmcblk";

// Linux has a constant sector size, this should never change.
const SECTOR_SIZE: u64 = 512;

pub struct DiskMetricsCollector<M: Mmc> {
    mmc: Vec<M>,
    prev_bytes_reading: Option<u64>,
}

impl<M> DiskMetricsCollector<M>
where
    M: Mmc,
{
    pub fn new(mmc: Vec<M>) -> Self {
        Self {
            mmc,
            prev_bytes_reading: None,
        }
    }

    fn get_disk_metrics(
        mmc: &M,
        disk_stats: Option<&Vec<u64>>,
        prev_bytes_reading: &mut Option<u64>,
    ) -> Result<Vec<KeyedMetricReading>> {
        let disk_name = mmc.disk_name();

        let mut metrics = vec![];

        if let Some(lifetime) = mmc.read_lifetime()? {
            let mlc_metric_key = MetricStringKey::from_str(&format!(
                "{}/{}/lifetime_pct",
                DISK_METRIC_NAMESPACE, disk_name
            ))
            .map_err(|e| eyre!("Invalid metric key: {}", e))?;

            let mlc_metric_reading =
                KeyedMetricReading::new_gauge(mlc_metric_key, lifetime.mlc_lifetime_pct as f64);

            metrics.push(mlc_metric_reading);
        }

        let bytes_written = disk_stats
            .and_then(|disk_stats| disk_stats.get(6))
            .map(|sectors_written| *sectors_written * SECTOR_SIZE);

        if let Some(bytes_written) = bytes_written {
            match Self::calc_bytes_written_reading(bytes_written, prev_bytes_reading, disk_name) {
                Ok(Some(reading)) => metrics.push(reading),
                Ok(None) => {}
                Err(e) => debug!("Failed to calculate bytes_written: {}", e),
            }
        }

        match mmc.product_name() {
            Ok(product_name) => {
                let product_name_metric_key = MetricStringKey::from_str(&format!(
                    "{}/{}/name",
                    DISK_METRIC_NAMESPACE, disk_name
                ))
                .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                let product_name_reading =
                    KeyedMetricReading::new_report_tag(product_name_metric_key, product_name);
                metrics.push(product_name_reading);
            }
            Err(e) => {
                debug!("Failed to read product name: {}", e)
            }
        }

        match mmc.manufacturer_id() {
            Ok(manufacturer_id) => {
                let manufacturer_id_metric_key = MetricStringKey::from_str(&format!(
                    "{}/{}/manufacturer_id",
                    DISK_METRIC_NAMESPACE, disk_name
                ))
                .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                let manufacturer_id_reading =
                    KeyedMetricReading::new_report_tag(manufacturer_id_metric_key, manufacturer_id);
                metrics.push(manufacturer_id_reading);
            }
            Err(e) => {
                debug!("Failed to read product name: {}", e)
            }
        }

        match mmc.disk_sector_count() {
            Ok(sector_count) => {
                let disk_size_metric_key = MetricStringKey::from_str(&format!(
                    "{}/{}/total_size_bytes",
                    DISK_METRIC_NAMESPACE, disk_name
                ))
                .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                let device_size = sector_count * SECTOR_SIZE;
                let sector_count_reading =
                    KeyedMetricReading::new_gauge(disk_size_metric_key, device_size as f64);
                metrics.push(sector_count_reading);
            }
            Err(e) => {
                debug!("Failed to read disk sector count: {}", e)
            }
        }

        match mmc.manufacture_date() {
            Ok(manufacture_date) => {
                let manufacture_date_metric_key = MetricStringKey::from_str(&format!(
                    "{}/{}/manufacture_date",
                    DISK_METRIC_NAMESPACE, disk_name
                ))
                .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                let manufacture_date_reading = KeyedMetricReading::new_report_tag(
                    manufacture_date_metric_key,
                    manufacture_date,
                );
                metrics.push(manufacture_date_reading);
            }
            Err(e) => {
                debug!("Failed to read manufacture date: {}", e)
            }
        }

        Ok(metrics)
    }

    fn calc_bytes_written_reading(
        cur_bytes_written: u64,
        prev_bytes_written: &mut Option<u64>,
        disk_name: &str,
    ) -> Result<Option<KeyedMetricReading>> {
        if let Some(prev_bytes_written) = prev_bytes_written.replace(cur_bytes_written) {
            match cur_bytes_written.checked_sub(prev_bytes_written) {
                Some(_) => {
                    let bytes_metric_key = MetricStringKey::from_str(&format!(
                        "{}/{}/bytes_written",
                        DISK_METRIC_NAMESPACE, disk_name
                    ))
                    .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                    let bytes_since_last_reading = cur_bytes_written - prev_bytes_written;
                    let bytes_metric = KeyedMetricReading::new_counter(
                        bytes_metric_key,
                        bytes_since_last_reading as f64,
                    );
                    Ok(Some(bytes_metric))
                }
                None => {
                    warn!("bytes_written metric overflow, discarding reading");
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }
}

impl<M> SystemMetricFamilyCollector for DiskMetricsCollector<M>
where
    M: Mmc,
{
    fn family_name(&self) -> &'static str {
        DISK_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let disk_stats_file = File::open(PROC_DISKSTATS_PATH)?;
        let disk_stats_reader = BufReader::new(disk_stats_file);

        let disk_stats_map = disk_stats_reader
            .lines()
            .filter_map(|line| {
                line.ok()
                    .and_then(|line| parse_proc_diskstats_line(&line).ok())
            })
            .collect::<HashMap<String, Vec<u64>>>();

        let metrics = self
            .mmc
            .iter()
            .filter_map(|m| {
                let disk_stats_line = disk_stats_map.get(m.disk_name());
                match Self::get_disk_metrics(m, disk_stats_line, &mut self.prev_bytes_reading) {
                    Ok(metrics) => Some(metrics),
                    Err(e) => {
                        error!(
                            "Failed to get MMC metrics for disk {}: {}",
                            m.disk_name(),
                            e
                        );
                        None
                    }
                }
            })
            .flatten()
            .collect();

        Ok(metrics)
    }
}

pub fn get_tracked_disks(
    value: DiskstatsMetricsConfig,
    sysfs_block_dir: &str,
) -> Result<Vec<String>> {
    let monitored_disks = match value {
        DiskstatsMetricsConfig::Auto => read_dir(sysfs_block_dir)?
            .filter_map(|dir| {
                let file_name_raw = dir.ok()?.file_name();
                let file_name = file_name_raw.to_string_lossy();

                file_name
                    .contains(TRACKED_DISK_PREFIX)
                    .then(|| format!("/dev/{}", file_name))
            })
            .collect(),
        DiskstatsMetricsConfig::Devices(devs) => devs
            .iter()
            .filter(|dev| dev.contains(TRACKED_DISK_PREFIX))
            .map(|dev| format!("/dev/{}", dev))
            .collect(),
    };

    Ok(monitored_disks)
}

#[cfg(test)]
mod test {
    use insta::{assert_json_snapshot, rounded_redaction};
    use rstest::rstest;
    use tempfile::tempdir;

    use super::*;

    use crate::mmc::{Mmc, MmcLifeTime, MmcType};

    #[derive(Clone)]
    struct FakeMmc {
        disk_name: String,
        product_name: String,
        lifetime: MmcLifeTime,
        manufacturer_id: String,
        sector_count: u64,
        manufacture_date: String,
    }

    impl Mmc for FakeMmc {
        fn disk_name(&self) -> &str {
            &self.disk_name
        }

        fn product_name(&self) -> Result<String> {
            Ok(self.product_name.clone())
        }

        fn read_lifetime(&self) -> Result<Option<MmcLifeTime>> {
            Ok(Some(self.lifetime.clone()))
        }

        fn disk_type(&self) -> MmcType {
            MmcType::Mmc
        }

        fn manufacturer_id(&self) -> Result<String> {
            Ok(self.manufacturer_id.clone())
        }

        fn disk_sector_count(&self) -> Result<u64> {
            Ok(self.sector_count)
        }

        fn manufacture_date(&self) -> Result<String> {
            Ok(self.manufacture_date.clone())
        }
    }

    #[test]
    fn test_get_disk_metrics() {
        // Create fake MMC
        let fake_mmc = FakeMmc {
            disk_name: "mmcblk0".to_string(),
            product_name: "SG123".to_string(),
            manufacturer_id: "0x00015".to_string(),
            lifetime: MmcLifeTime {
                mlc_lifetime_pct: 90,
                slc_lifetime_pct: 85,
            },
            sector_count: 100,
            manufacture_date: "11/2023".to_string(),
        };

        // Create disk stats (sectors written = 1000)
        let disk_stats = vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0];
        let mut prev_bytes_reading = None;

        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&disk_stats),
            &mut prev_bytes_reading,
        )
        .unwrap();

        assert_eq!(metrics.len(), 5);

        // Create updated disk stats (sectors written = 2000)
        let updated_disk_stats = vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0];

        // Second call should include bytes written metric
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&updated_disk_stats),
            &mut prev_bytes_reading,
        )
        .unwrap();

        assert_eq!(metrics.len(), 6);
        assert_json_snapshot!(metrics, {
            "[].value.**.timestamp" => "[timestamp]",
            "[].value.**.value" => rounded_redaction(5)
        });
    }

    #[test]
    fn test_calc_bytes_reading_overflow() {
        let mut prev_bytes_reading = Some(1000);
        let cur_bytes_written = 500;

        let result = DiskMetricsCollector::<FakeMmc>::calc_bytes_written_reading(
            cur_bytes_written,
            &mut prev_bytes_reading,
            "mmcblk0",
        )
        .unwrap();

        assert!(result.is_none());
    }

    #[rstest]
    #[case(DiskstatsMetricsConfig::Auto)]
    #[case(DiskstatsMetricsConfig::Devices(
        vec!["mmcblk0".to_string()].into_iter().collect()
    ))]
    fn test_get_tracked_disks(#[case] diskstats_config: DiskstatsMetricsConfig) {
        let temp_dir = tempdir().unwrap();
        let temp_dir_path = temp_dir.path();

        let mmc_path = temp_dir_path.join("mmcblk0");
        let nvme_path = temp_dir_path.join("nvme0n1");

        let _ = File::create(mmc_path).unwrap();
        let _ = File::create(nvme_path).unwrap();

        let tracked_disks = get_tracked_disks(diskstats_config, temp_dir_path.to_str().unwrap())
            .expect("Failed to get tracked disks");

        assert_eq!(tracked_disks.len(), 1);
        assert_eq!(tracked_disks[0], "/dev/mmcblk0");
    }
}
