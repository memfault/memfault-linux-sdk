//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect temperature readings from /sys/class/thermal
//!
//! This module parses thermal readings from /sys/class/thermal and constructs
//! KeyedMetricReadings based on those statistics.
//!
//! Example /sys/class/thermal contents:
//!
//! /sys/class/thermal/
//! ├── cooling_device0 -> ../../devices/virtual/thermal/cooling_device0
//! ├── cooling_device1 -> ../../devices/virtual/thermal/cooling_device1
//! ├── thermal_zone0 -> ../../devices/virtual/thermal/thermal_zone0
//! └── thermal_zone1 -> ../../devices/virtual/thermal/thermal_zone1
//!
//! Example /sys/class/thermal/thermal_zone[0-*] contents:
//!
//! /sys/class/thermal/thermal_zone0
//! ├── ...
//! └── temp  // this is the property we're interested in
//!
//! See additional Linux kernel documentation on /sys/class/thermal here:
//! https://www.kernel.org/doc/Documentation/thermal/sysfs-api.txt

use std::{fs::read_to_string, str::FromStr};

use crate::metrics::{
    system_metrics::SystemMetricFamilyCollector, KeyedMetricReading, MetricStringKey,
};
use eyre::{eyre, Result};

const SYS_CLASS_THERMAL_PATH: &str = "/sys/class/thermal";
pub const THERMAL_METRIC_NAMESPACE: &str = "thermal";

pub struct ThermalMetricsCollector;

impl ThermalMetricsCollector {
    pub fn new() -> Self {
        ThermalMetricsCollector {}
    }

    fn parse_thermal_zone_temp(temp: &str, thermal_zone_type: &str) -> Result<KeyedMetricReading> {
        // The readings are in millidegrees Celsius, so we divide by 1000 to get
        // the temperature in degrees Celsius.
        let temp_in_celsius = temp.trim().parse::<f64>()? / 1000.0;

        Ok(KeyedMetricReading::new_histogram(
            MetricStringKey::from_str(
                format!(
                    "{}/{}/temp",
                    THERMAL_METRIC_NAMESPACE,
                    thermal_zone_type.trim()
                )
                .as_str(),
            )
            .map_err(|e| {
                eyre!(
                    "Failed to construct MetricStringKey for thermal zone: {}",
                    e
                )
            })?,
            temp_in_celsius,
        ))
    }

    fn read_thermal_zone_temp(root_dir: &str, zone_name: &str) -> Result<ThermalZoneTemp> {
        let temp_file = format!("{}/{}/temp", root_dir, zone_name);
        let type_file = format!("{}/{}/type", root_dir, zone_name);

        let temp = read_to_string(temp_file)?.trim().to_string();
        let zone = read_to_string(type_file)?.trim().to_string();

        Ok(ThermalZoneTemp { temp, zone })
    }

    // To facilitate unit testing, make the thermal directory path an arg
    fn read_thermal_metrics_from_dir(dir: &str) -> Result<Vec<ThermalZoneTemp>> {
        // The /sys/class/thermal/ directory will contain symlinks to
        // pseudo-files named "thermal_zone0" etc, depending on the number of
        // thermal zones in the system. The file we read for the temperature
        // reading is for example /sys/class/thermal/thermal_zone0/temp,
        // containing an integer value in millidegrees Celsius, ex: "53000"
        let zone_temps: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter_map(|path| Some(path.file_name()?.to_str()?.to_string()))
            .filter(|name| name.starts_with("thermal_zone"))
            .filter_map(|name| Self::read_thermal_zone_temp(dir, &name).ok())
            .collect();

        Ok(zone_temps)
    }

    pub fn get_thermal_metrics() -> Result<Vec<KeyedMetricReading>> {
        let zone_temps = Self::read_thermal_metrics_from_dir(SYS_CLASS_THERMAL_PATH)?;

        zone_temps
            .into_iter()
            .map(|zone_temp| Self::parse_thermal_zone_temp(&zone_temp.temp, &zone_temp.zone))
            .collect()
    }
}

impl SystemMetricFamilyCollector for ThermalMetricsCollector {
    fn family_name(&self) -> &'static str {
        THERMAL_METRIC_NAMESPACE
    }

    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        Self::get_thermal_metrics()
    }
}

struct ThermalZoneTemp {
    temp: String,
    zone: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::{assert_json_snapshot, rounded_redaction};
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use tempfile::{tempdir, TempDir};

    fn write_thermal_zone(root_dir: &Path, zone_name: &str, temp: &str, zone_type: &str) {
        let thermal_zone_dir = root_dir.join(zone_name);
        std::fs::create_dir(&thermal_zone_dir).unwrap();

        let mut temp_file = File::create(thermal_zone_dir.join("temp")).unwrap();
        writeln!(temp_file, "{}", temp).unwrap();

        let mut type_file = File::create(thermal_zone_dir.join("type")).unwrap();
        writeln!(type_file, "{}", zone_type).unwrap();
    }

    fn thermal_dir_with_zones(zones: &[(&str, &str, &str)]) -> TempDir {
        let dir = tempdir().unwrap();
        for (zone_name, temp, zone_type) in zones {
            write_thermal_zone(dir.path(), zone_name, temp, zone_type);
        }
        dir
    }

    #[test]
    fn test_parse_thermal_zone_temp() {
        let result = ThermalMetricsCollector::parse_thermal_zone_temp("50000", "cpu-temp").unwrap();

        assert_json_snapshot!(result, {".value.**.timestamp" => "[timestamp]", ".value.**.value" => rounded_redaction(5)});
    }

    #[test]
    fn test_parse_thermal_zone_temp_trims_input() {
        let result =
            ThermalMetricsCollector::parse_thermal_zone_temp("50000\n", "cpu-temp\n").unwrap();

        assert_json_snapshot!(result, {".value.**.timestamp" => "[timestamp]", ".value.**.value" => rounded_redaction(5)});
    }

    #[test]
    fn test_parse_thermal_zone_temp_invalid_temp() {
        assert!(
            ThermalMetricsCollector::parse_thermal_zone_temp("not-a-number", "cpu-temp").is_err()
        )
    }

    #[test]
    fn test_read_thermal_zone_temp() {
        let dir = thermal_dir_with_zones(&[("thermal_zone0", "50000", "cpu-temp")]);

        let result = ThermalMetricsCollector::read_thermal_zone_temp(
            dir.path().to_str().unwrap(),
            "thermal_zone0",
        )
        .unwrap();

        assert_eq!(result.temp, "50000");
        assert_eq!(result.zone, "cpu-temp");

        dir.close().unwrap();
    }

    #[test]
    fn test_read_thermal_zone_temp_missing_zone() {
        let dir = tempdir().unwrap();

        assert!(ThermalMetricsCollector::read_thermal_zone_temp(
            dir.path().to_str().unwrap(),
            "thermal_zone0",
        )
        .is_err());

        dir.close().unwrap();
    }

    #[test]
    fn test_read_thermal_metrics_from_dir() {
        let dir = thermal_dir_with_zones(&[
            ("thermal_zone0", "50000", "cpu-temp"),
            ("thermal_zone1", "37500", "gpu-temp"),
        ]);
        // Non-thermal_zone entries in /sys/class/thermal must be ignored
        std::fs::create_dir(dir.path().join("cooling_device0")).unwrap();

        let mut zone_temps =
            ThermalMetricsCollector::read_thermal_metrics_from_dir(dir.path().to_str().unwrap())
                .unwrap();
        // read_dir ordering is not guaranteed
        zone_temps.sort_by(|a, b| a.zone.cmp(&b.zone));

        let zone_temps: Vec<_> = zone_temps
            .into_iter()
            .map(|zone_temp| (zone_temp.zone, zone_temp.temp))
            .collect();
        assert_eq!(
            zone_temps,
            vec![
                ("cpu-temp".to_string(), "50000".to_string()),
                ("gpu-temp".to_string(), "37500".to_string()),
            ]
        );

        dir.close().unwrap();
    }

    #[test]
    fn test_read_thermal_metrics_from_dir_skips_unreadable_zone() {
        let dir = thermal_dir_with_zones(&[("thermal_zone0", "50000", "cpu-temp")]);
        // A thermal zone with no temp/type files should be skipped, not fail the read
        std::fs::create_dir(dir.path().join("thermal_zone1")).unwrap();

        let zone_temps =
            ThermalMetricsCollector::read_thermal_metrics_from_dir(dir.path().to_str().unwrap())
                .unwrap();

        assert_eq!(zone_temps.len(), 1);
        assert_eq!(zone_temps[0].zone, "cpu-temp");

        dir.close().unwrap();
    }

    #[test]
    fn test_read_thermal_metrics_from_dir_missing_dir() {
        assert!(ThermalMetricsCollector::read_thermal_metrics_from_dir(
            "/nonexistent/sys/class/thermal"
        )
        .is_err())
    }
}
