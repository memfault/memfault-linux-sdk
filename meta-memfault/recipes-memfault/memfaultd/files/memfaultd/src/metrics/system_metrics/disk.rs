//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashMap,
    fs::{read_dir, File},
    io::{BufRead, BufReader},
    str::FromStr,
    time::{Duration, Instant},
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
    last_lifetime_reading: Option<Instant>,
}

impl<M> DiskMetricsCollector<M>
where
    M: Mmc,
{
    // Only read lifetime once an hour
    const LIFETIME_READING_INTERVAL: Duration = Duration::from_secs(3600);

    pub fn new(mmc: Vec<M>) -> Self {
        Self {
            mmc,
            prev_bytes_reading: None,
            last_lifetime_reading: None,
        }
    }

    fn get_lifetime_readings(disk_name: &str, mmc: &M) -> Result<Vec<KeyedMetricReading>> {
        let mut metrics = Vec::with_capacity(2);
        if let Some(lifetime) = mmc.read_lifetime()? {
            match lifetime.lifetime_a_pct {
                Some(lifetime_a_pct) => {
                    let lifetime_a_metric_key = MetricStringKey::from_str(&format!(
                        "{}/{}/lifetime_remaining_pct",
                        DISK_METRIC_NAMESPACE, disk_name
                    ))
                    .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                    let lifetime_a_metric_reading =
                        100u8.checked_sub(lifetime_a_pct).map(|pct_remaining| {
                            KeyedMetricReading::new_gauge(
                                lifetime_a_metric_key,
                                pct_remaining as f64,
                            )
                        });

                    match lifetime_a_metric_reading {
                        Some(reading) => metrics.push(reading),
                        None => debug!("Underflow - lifetime a greater than 100"),
                    }
                }
                None => debug!("Invalid lifetime a pct"),
            }

            match lifetime.lifetime_b_pct {
                Some(lifetime_b_pct) => {
                    let lifetime_b_metric_key = MetricStringKey::from_str(&format!(
                        "{}/{}/lifetime_b_remaining_pct",
                        DISK_METRIC_NAMESPACE, disk_name
                    ))
                    .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                    let lifetime_b_metric_reading =
                        100u8.checked_sub(lifetime_b_pct).map(|pct_remaining| {
                            KeyedMetricReading::new_gauge(
                                lifetime_b_metric_key,
                                pct_remaining as f64,
                            )
                        });

                    match lifetime_b_metric_reading {
                        Some(reading) => metrics.push(reading),
                        None => debug!("Underflow - lifetime b greater than 100"),
                    }
                }
                None => debug!("Invalid lifetime b pct"),
            }
        }

        Ok(metrics)
    }

    fn get_disk_metrics(
        mmc: &M,
        disk_stats: Option<&Vec<u64>>,
        prev_bytes_reading: &mut Option<u64>,
        last_lifetime_reading: &mut Option<Instant>,
    ) -> Result<Vec<KeyedMetricReading>> {
        let disk_name = mmc.disk_name();

        let mut metrics = vec![];

        match last_lifetime_reading {
            Some(last_reading) => {
                let now = Instant::now();
                let get_next_reading =
                    now.checked_duration_since(*last_reading)
                        .is_some_and(|duration_since| {
                            duration_since >= Self::LIFETIME_READING_INTERVAL
                        });

                if get_next_reading {
                    metrics.extend(Self::get_lifetime_readings(disk_name, mmc)?);
                    *last_lifetime_reading = Some(now);
                }
            }
            None => {
                metrics.extend(Self::get_lifetime_readings(disk_name, mmc)?);
                *last_lifetime_reading = Some(Instant::now());
            }
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

        match mmc.revision() {
            Ok(revision) => {
                let revision_metric_key = MetricStringKey::from_str(&format!(
                    "{}/{}/revision",
                    DISK_METRIC_NAMESPACE, disk_name
                ))
                .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                let revision_reading =
                    KeyedMetricReading::new_report_tag(revision_metric_key, revision);
                metrics.push(revision_reading);
            }
            Err(e) => {
                debug!("Failed to read revision: {}", e)
            }
        }

        match mmc.serial() {
            Ok(serial) => {
                let serial_metric_key = MetricStringKey::from_str(&format!(
                    "{}/{}/serial",
                    DISK_METRIC_NAMESPACE, disk_name
                ))
                .map_err(|e| eyre!("Invalid metric key: {}", e))?;
                let serial_reading = KeyedMetricReading::new_report_tag(serial_metric_key, serial);
                metrics.push(serial_reading);
            }
            Err(e) => {
                debug!("Failed to read serial: {}", e)
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
                match Self::get_disk_metrics(
                    m,
                    disk_stats_line,
                    &mut self.prev_bytes_reading,
                    &mut self.last_lifetime_reading,
                ) {
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
        revision: String,
        serial: String,
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

        fn revision(&self) -> Result<String> {
            Ok(self.revision.clone())
        }

        fn serial(&self) -> Result<String> {
            Ok(self.serial.clone())
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
                lifetime_a_pct: Some(90),
                lifetime_b_pct: Some(85),
            },
            sector_count: 100,
            manufacture_date: "11/2023".to_string(),
            revision: "1.0".to_string(),
            serial: "0x1234567890".to_string(),
        };

        // Create disk stats (sectors written = 1000)
        let disk_stats = vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0];
        let mut prev_bytes_reading = None;

        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;
        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&disk_stats),
            &mut prev_bytes_reading,
            &mut Some(hour_ago),
        )
        .unwrap();

        assert_eq!(metrics.len(), 8);

        // Create updated disk stats (sectors written = 2000)
        let updated_disk_stats = vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0];

        // Second call should include bytes written metric
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&updated_disk_stats),
            &mut prev_bytes_reading,
            &mut Some(hour_ago),
        )
        .unwrap();

        assert_eq!(metrics.len(), 9);
        assert_json_snapshot!(metrics, {
            "[].value.**.timestamp" => "[timestamp]",
            "[].value.**.value" => rounded_redaction(5)
        });
    }

    #[test]
    fn test_get_disk_metrics_without_lifetimes() {
        // Create fake MMC
        let fake_mmc = FakeMmc {
            disk_name: "mmcblk0".to_string(),
            product_name: "SG123".to_string(),
            manufacturer_id: "0x00015".to_string(),
            lifetime: MmcLifeTime {
                lifetime_a_pct: None,
                lifetime_b_pct: None,
            },
            sector_count: 100,
            manufacture_date: "11/2023".to_string(),
            revision: "1.0".to_string(),
            serial: "0x1234567890".to_string(),
        };

        // Create disk stats (sectors written = 1000)
        let disk_stats = vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0];
        let mut prev_bytes_reading = None;
        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;

        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&disk_stats),
            &mut prev_bytes_reading,
            &mut Some(hour_ago),
        )
        .unwrap();

        assert_eq!(metrics.len(), 6);

        // Create updated disk stats (sectors written = 2000)
        let updated_disk_stats = vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0];

        // Second call should include bytes written metric
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&updated_disk_stats),
            &mut prev_bytes_reading,
            &mut Some(hour_ago),
        )
        .unwrap();

        assert_eq!(metrics.len(), 7);
        assert_json_snapshot!(metrics, {
            "[].value.**.timestamp" => "[timestamp]",
            "[].value.**.value" => rounded_redaction(5)
        });
    }

    #[test]
    fn test_lifetime_metric_underflow() {
        // Create fake MMC
        let fake_mmc = FakeMmc {
            disk_name: "mmcblk0".to_string(),
            product_name: "SG123".to_string(),
            manufacturer_id: "0x00015".to_string(),
            lifetime: MmcLifeTime {
                lifetime_a_pct: Some(120),
                lifetime_b_pct: Some(80),
            },
            sector_count: 100,
            manufacture_date: "11/2023".to_string(),
            revision: "1.0".to_string(),
            serial: "0x1234567890".to_string(),
        };

        // Create disk stats (sectors written = 1000)
        let disk_stats = vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0];
        let mut prev_bytes_reading = None;
        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;

        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            Some(&disk_stats),
            &mut prev_bytes_reading,
            &mut Some(hour_ago),
        )
        .unwrap();

        assert_eq!(metrics.len(), 7);
        assert_json_snapshot!(metrics, {
            "[].value.**.timestamp" => "[timestamp]",
            "[].value.**.value" => rounded_redaction(5)
        });
    }

    #[test]
    fn test_lifetime_interval_update() {
        let fake_mmc = FakeMmc {
            disk_name: "mmcblk0".to_string(),
            product_name: "SG123".to_string(),
            manufacturer_id: "0x00015".to_string(),
            lifetime: MmcLifeTime {
                lifetime_a_pct: Some(90),
                lifetime_b_pct: Some(85),
            },
            sector_count: 100,
            manufacture_date: "11/2023".to_string(),
            revision: "1.0".to_string(),
            serial: "0x1234567890".to_string(),
        };

        let mut prev_bytes_reading = None;
        let mut last_lifetime_reading = None;

        // First reading should set the last reading time
        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            None,
            &mut prev_bytes_reading,
            &mut last_lifetime_reading,
        )
        .unwrap();

        assert_eq!(metrics.len(), 8);
        assert!(last_lifetime_reading.is_some());
    }

    #[test]
    fn test_lifetime_interval_read() {
        let fake_mmc = FakeMmc {
            disk_name: "mmcblk0".to_string(),
            product_name: "SG123".to_string(),
            manufacturer_id: "0x00015".to_string(),
            lifetime: MmcLifeTime {
                lifetime_a_pct: Some(90),
                lifetime_b_pct: Some(85),
            },
            sector_count: 100,
            manufacture_date: "11/2023".to_string(),
            revision: "1.0".to_string(),
            serial: "0x1234567890".to_string(),
        };

        let mut prev_bytes_reading = None;
        let mut last_lifetime_reading =
            Some(Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL);

        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            None,
            &mut prev_bytes_reading,
            &mut last_lifetime_reading,
        )
        .unwrap();

        // Should read lifetime metrics again since the interval has passed
        assert_eq!(metrics.len(), 8);

        let mut prev_bytes_reading = None;
        let mut last_lifetime_reading = Some(Instant::now());

        let metrics = DiskMetricsCollector::get_disk_metrics(
            &fake_mmc,
            None,
            &mut prev_bytes_reading,
            &mut last_lifetime_reading,
        )
        .unwrap();

        // Should not read lifetime metrics again since the interval has not passed
        assert_eq!(metrics.len(), 6);
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
