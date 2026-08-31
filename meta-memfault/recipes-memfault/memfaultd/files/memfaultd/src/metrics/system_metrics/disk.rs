//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashMap,
    fs::{read_dir, read_to_string},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use eyre::{eyre, Result};
use log::{debug, error, warn};

use crate::{
    metrics::{KeyedMetricReading, MetricStringKey},
    mmc::{Mmc, MmcLifeTime},
};

use super::diskstats::DiskstatsMetricsConfig;
use super::{diskstats::parse_proc_diskstats_line, SystemMetricFamilyCollector};

const PROC_DISKSTATS_PATH: &str = "/proc/diskstats";
pub const DISK_METRIC_NAMESPACE: &str = "diskstats";

const TRACKED_DISK_PREFIX: &str = "mmcblk";

// Linux has a constant sector size, this should never change.
const SECTOR_SIZE: u64 = 512;

/// Values read directly off an MMC device, gathered up front so the readings
/// can be built afterwards without borrowing `prev_sector_readings` or
/// `last_lifetime_readings` during the read.
struct DiskRawReadings {
    disk_name: String,
    // `None` when this cycle didn't attempt a lifetime read (throttled).
    lifetime: Option<Result<Option<MmcLifeTime>>>,
    product_name: Result<String>,
    manufacturer_id: Result<String>,
    sector_count: Result<u64>,
    manufacture_date: Result<String>,
    revision: Result<String>,
    serial: Result<String>,
}

pub struct DiskMetricsCollector<M: Mmc> {
    mmc: Arc<Vec<M>>,
    prev_sector_readings: HashMap<String, u64>,
    last_lifetime_readings: HashMap<String, Instant>,
}

impl<M> DiskMetricsCollector<M>
where
    M: Mmc + Send + 'static,
{
    // Only read lifetime once an hour
    const LIFETIME_READING_INTERVAL: Duration = Duration::from_secs(3600);
    const SECTORS_WRITTEN_DISKSTATS_OFFSET: usize = 6;

    pub fn new(mmc: Vec<M>) -> Self {
        let mmc = Arc::new(mmc);
        Self {
            mmc,
            prev_sector_readings: HashMap::new(),
            last_lifetime_readings: HashMap::new(),
        }
    }

    fn collect(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let disk_stats = read_to_string(PROC_DISKSTATS_PATH)?;

        let disk_stats_map = disk_stats
            .lines()
            .filter_map(|line| parse_proc_diskstats_line(line).ok())
            .collect::<HashMap<String, Vec<u64>>>();

        let should_read_lifetime = self
            .mmc
            .iter()
            .map(|m| {
                let disk_name = m.disk_name();
                let should_read =
                    Self::should_read_lifetime(disk_name, &self.last_lifetime_readings);
                (disk_name.to_string(), should_read)
            })
            .collect::<HashMap<String, bool>>();

        let mmc = self.mmc.clone();
        let raw_readings = Self::read_all_disk_values(mmc, should_read_lifetime);

        let metrics = raw_readings
            .into_iter()
            .filter_map(|raw| {
                let disk_name = raw.disk_name.clone();
                match self.build_disk_metrics(raw, &disk_stats_map) {
                    Ok(metrics) => Some(metrics),
                    Err(e) => {
                        error!("Failed to get MMC metrics for disk {}: {}", disk_name, e);
                        None
                    }
                }
            })
            .flatten()
            .collect();

        Ok(metrics)
    }

    fn should_read_lifetime(
        disk_name: &str,
        last_lifetime_reading: &HashMap<String, Instant>,
    ) -> bool {
        match last_lifetime_reading.get(disk_name) {
            Some(last_reading) => Instant::now()
                .checked_duration_since(*last_reading)
                .is_some_and(|duration_since| duration_since >= Self::LIFETIME_READING_INTERVAL),
            None => true,
        }
    }

    fn read_disk_raw_values(mmc: &M, should_read_lifetime: bool) -> DiskRawReadings {
        DiskRawReadings {
            disk_name: mmc.disk_name().to_string(),
            lifetime: should_read_lifetime.then(|| mmc.read_lifetime()),
            product_name: mmc.product_name(),
            manufacturer_id: mmc.manufacturer_id(),
            sector_count: mmc.disk_sector_count(),
            manufacture_date: mmc.manufacture_date(),
            revision: mmc.revision(),
            serial: mmc.serial(),
        }
    }

    fn read_all_disk_values(
        mmc: Arc<Vec<M>>,
        should_read_lifetime: HashMap<String, bool>,
    ) -> Vec<DiskRawReadings> {
        mmc.iter()
            .map(|m| {
                let should_read = should_read_lifetime
                    .get(m.disk_name())
                    .copied()
                    .unwrap_or(true);
                Self::read_disk_raw_values(m, should_read)
            })
            .collect()
    }

    fn lifetime_readings(
        disk_name: &str,
        lifetime_result: Result<Option<MmcLifeTime>>,
    ) -> Result<Vec<KeyedMetricReading>> {
        let mut metrics = Vec::with_capacity(2);
        if let Some(lifetime) = lifetime_result? {
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

    fn build_disk_metrics(
        &mut self,
        raw: DiskRawReadings,
        disk_stats_map: &HashMap<String, Vec<u64>>,
    ) -> Result<Vec<KeyedMetricReading>> {
        let disk_name = raw.disk_name;

        let mut metrics = vec![];

        // If a lifetime read was attempted this cycle, update the throttle
        // timestamp only once the readings have been built successfully -
        // matching the previous behavior of retrying every cycle after a
        // failed read rather than waiting out the full interval again.
        if let Some(lifetime_result) = raw.lifetime {
            metrics.extend(Self::lifetime_readings(&disk_name, lifetime_result)?);
            self.last_lifetime_readings
                .insert(disk_name.clone(), Instant::now());
        }

        let sectors_written = disk_stats_map
            .get(&disk_name)
            .and_then(|disk_stats| disk_stats.get(Self::SECTORS_WRITTEN_DISKSTATS_OFFSET));

        if let Some(sectors_written) = sectors_written {
            match Self::calc_bytes_written_reading(
                *sectors_written,
                &mut self.prev_sector_readings,
                &disk_name,
            ) {
                Ok(Some(reading)) => metrics.push(reading),
                Ok(None) => {}
                Err(e) => debug!("Failed to calculate bytes_written: {}", e),
            }
        }

        match raw.product_name {
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

        match raw.manufacturer_id {
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

        match raw.sector_count {
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

        match raw.manufacture_date {
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

        match raw.revision {
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

        match raw.serial {
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
        cur_sectors_written: u64,
        prev_sector_readings: &mut HashMap<String, u64>,
        disk_name: &str,
    ) -> Result<Option<KeyedMetricReading>> {
        if let Some(prev_sectors_written) =
            prev_sector_readings.insert(disk_name.to_string(), cur_sectors_written)
        {
            match cur_sectors_written
                .checked_sub(prev_sectors_written)
                .and_then(|sectors| sectors.checked_mul(SECTOR_SIZE))
            {
                Some(bytes_since_last_reading) => {
                    let bytes_metric_key = MetricStringKey::from_str(&format!(
                        "{}/{}/bytes_written",
                        DISK_METRIC_NAMESPACE, disk_name
                    ))
                    .map_err(|e| eyre!("Invalid metric key: {}", e))?;

                    let bytes_metric = KeyedMetricReading::new_counter(
                        bytes_metric_key,
                        bytes_since_last_reading as f64,
                    );
                    Ok(Some(bytes_metric))
                }
                None => {
                    warn!(
                        "bytes_written metric overflow for disk {}, discarding reading",
                        disk_name
                    );
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
    M: Mmc + Send + 'static,
{
    fn family_name(&self) -> &'static str {
        DISK_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.collect()
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
    use std::fs::File;

    use insta::{assert_json_snapshot, rounded_redaction};
    use rstest::rstest;
    use tempfile::tempdir;

    use super::*;

    use crate::mmc::{Mmc, MmcLifeTime};

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

    /// Mirrors what `collect()` does for a single disk: decide whether to
    /// read lifetime, read the raw values, then build readings from them.
    fn get_disk_metrics(
        collector: &mut DiskMetricsCollector<FakeMmc>,
        mmc: &FakeMmc,
        disk_stats_map: &HashMap<String, Vec<u64>>,
    ) -> Result<Vec<KeyedMetricReading>> {
        let should_read_lifetime = DiskMetricsCollector::<FakeMmc>::should_read_lifetime(
            mmc.disk_name(),
            &collector.last_lifetime_readings,
        );
        let raw = DiskMetricsCollector::read_disk_raw_values(mmc, should_read_lifetime);
        collector.build_disk_metrics(raw, disk_stats_map)
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

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc.clone()]);

        // Create disk stats (sectors written = 1000)
        let mut disk_stats_map = HashMap::new();
        disk_stats_map.insert(
            fake_mmc.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0],
        );

        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), hour_ago);

        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &disk_stats_map).unwrap();

        assert_eq!(metrics.len(), 8);

        // Create updated disk stats (sectors written = 2000)
        let mut updated_disk_stats_map = HashMap::new();
        updated_disk_stats_map.insert(
            fake_mmc.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0],
        );

        // Second call should include bytes written metric
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), hour_ago);
        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &updated_disk_stats_map).unwrap();

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

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc.clone()]);

        // Create disk stats (sectors written = 1000)
        let mut disk_stats_map = HashMap::new();
        disk_stats_map.insert(
            fake_mmc.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0],
        );

        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), hour_ago);

        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &disk_stats_map).unwrap();

        assert_eq!(collector.prev_sector_readings.get("mmcblk0"), Some(&1000));
        assert_eq!(metrics.len(), 6);

        // Create updated disk stats (sectors written = 2000)
        let mut updated_disk_stats_map = HashMap::new();
        updated_disk_stats_map.insert(
            fake_mmc.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0],
        );

        // Second call should include bytes written metric
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), hour_ago);
        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &updated_disk_stats_map).unwrap();

        assert_eq!(collector.prev_sector_readings.get("mmcblk0"), Some(&2000));
        assert_eq!(metrics.len(), 7);
        assert_json_snapshot!(metrics, {
            "[].value.**.timestamp" => "[timestamp]",
            "[].value.**.value" => rounded_redaction(5)
        });
    }

    #[test]
    fn get_disk_metrics_multiple_disks() {
        // Create fake MMCs
        let fake_mmc1 = FakeMmc {
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

        let fake_mmc2 = FakeMmc {
            disk_name: "mmcblk1".to_string(),
            product_name: "SG456".to_string(),
            manufacturer_id: "0x00016".to_string(),
            lifetime: MmcLifeTime {
                lifetime_a_pct: Some(95),
                lifetime_b_pct: Some(90),
            },
            sector_count: 200,
            manufacture_date: "12/2023".to_string(),
            revision: "1.1".to_string(),
            serial: "0x0987654321".to_string(),
        };

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc1.clone(), fake_mmc2.clone()]);

        let mut disk_stats_map = HashMap::new();
        disk_stats_map.insert(
            fake_mmc1.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0],
        );
        disk_stats_map.insert(
            fake_mmc2.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0],
        );

        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;

        let metrics1 = get_disk_metrics(&mut collector, &fake_mmc1, &disk_stats_map).unwrap();
        assert_eq!(metrics1.len(), 8);

        let metrics2 = get_disk_metrics(&mut collector, &fake_mmc2, &disk_stats_map).unwrap();
        assert_eq!(metrics2.len(), 8);

        let mut new_disk_stats_map = HashMap::new();
        new_disk_stats_map.insert(
            fake_mmc1.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0],
        );
        new_disk_stats_map.insert(
            fake_mmc2.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 2000, 0, 0, 0, 0],
        );
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), hour_ago);
        collector
            .last_lifetime_readings
            .insert("mmcblk1".to_string(), hour_ago);

        let metrics1 = get_disk_metrics(&mut collector, &fake_mmc1, &new_disk_stats_map).unwrap();
        assert_eq!(metrics1.len(), 9);

        let metrics2 = get_disk_metrics(&mut collector, &fake_mmc2, &new_disk_stats_map).unwrap();
        assert_eq!(metrics2.len(), 9);
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

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc.clone()]);

        // Create disk stats (sectors written = 1000)
        let mut disk_stats_map = HashMap::new();
        disk_stats_map.insert(
            fake_mmc.disk_name.clone(),
            vec![0, 0, 0, 0, 0, 0, 1000, 0, 0, 0, 0],
        );

        let hour_ago = Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL;
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), hour_ago);

        // First call should return MLC and SLC metrics but no bytes written (no previous reading)
        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &disk_stats_map).unwrap();

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

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc.clone()]);

        // First reading should set the last reading time
        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &HashMap::new()).unwrap();

        assert_eq!(metrics.len(), 8);
        assert!(collector.last_lifetime_readings.contains_key("mmcblk0"));
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

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc.clone()]);
        collector.last_lifetime_readings.insert(
            "mmcblk0".to_string(),
            Instant::now() - DiskMetricsCollector::<FakeMmc>::LIFETIME_READING_INTERVAL,
        );

        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &HashMap::new()).unwrap();

        // Should read lifetime metrics again since the interval has passed
        assert_eq!(metrics.len(), 8);

        let mut collector = DiskMetricsCollector::new(vec![fake_mmc.clone()]);
        collector
            .last_lifetime_readings
            .insert("mmcblk0".to_string(), Instant::now());

        let metrics = get_disk_metrics(&mut collector, &fake_mmc, &HashMap::new()).unwrap();

        // Should not read lifetime metrics again since the interval has not passed
        assert_eq!(metrics.len(), 6);
    }

    #[rstest]
    #[case(Some(1000), 500, None)]
    #[case(Some(200), 500, Some(300 * SECTOR_SIZE))]
    #[case(Some(1000), 1000, Some(0))]
    #[case(None, 500, None)]
    #[case(Some(0), u64::MAX, None)]
    fn test_calc_bytes_reading_overflow(
        #[case] prev_sectors: Option<u64>,
        #[case] cur_bytes_written: u64,
        #[case] diff: Option<u64>,
    ) {
        let mut prev_sector_readings = HashMap::new();
        if let Some(prev) = prev_sectors {
            prev_sector_readings.insert("mmcblk0".to_string(), prev);
        }
        let result = DiskMetricsCollector::<FakeMmc>::calc_bytes_written_reading(
            cur_bytes_written,
            &mut prev_sector_readings,
            "mmcblk0",
        )
        .unwrap();

        match diff {
            Some(diff) => {
                assert!(result.is_some());
                let reading = result.unwrap();
                let value = match reading.value {
                    crate::metrics::MetricReading::Counter { value, .. } => value as u64,
                    _ => panic!("Expected counter reading"),
                };

                assert_eq!(value, diff);
            }
            None => assert!(result.is_none()),
        }
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
