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

    fn read_thermal_zone_temp(zone_name: &str, root_dir: &str) -> Result<KeyedMetricReading> {
        let temp_file = format!("{}/{}/temp", root_dir, zone_name);
        let type_file = format!("{}/{}/type", root_dir, zone_name);

        // The readings are in millidegrees Celsius, so we divide by 1000 to get
        // the temperature in degrees Celsius.
        let temp_in_celsius = read_to_string(temp_file)?.trim().parse::<f64>()? / 1000.0;

        // Extract the string for the type of the thermal zone for use in the metric key
        let thermal_zone_type = read_to_string(type_file)?.trim().to_string();

        Ok(KeyedMetricReading::new_histogram(
            MetricStringKey::from_str(
                format!("{}/{}/temp", THERMAL_METRIC_NAMESPACE, thermal_zone_type).as_str(),
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

    // To facilitate unit testing, make the thermal directory path an arg
    fn read_thermal_metrics_from_dir(dir: &str) -> Result<Vec<KeyedMetricReading>> {
        // The /sys/class/thermal/ directory will contain symlinks to
        // pseudo-files named "thermal_zone0" etc, depending on the number of
        // thermal zones in the system. The file we read for the temperature
        // reading is for example /sys/class/thermal/thermal_zone0/temp,
        // containing an integer value in millidegrees Celsius, ex: "53000"
        let metrics: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter_map(|path| Some(path.file_name()?.to_str()?.to_string()))
            .filter(|name| name.starts_with("thermal_zone"))
            .filter_map(|name| Self::read_thermal_zone_temp(&name, dir).ok())
            .collect();

        Ok(metrics)
    }

    pub fn get_thermal_metrics() -> Result<Vec<KeyedMetricReading>> {
        Self::read_thermal_metrics_from_dir(SYS_CLASS_THERMAL_PATH)
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

#[cfg(test)]
// The floating point literal pattern is allowed in this test module because
// the input and output values are known.
mod tests {
    use super::*;
    use insta::{assert_json_snapshot, rounded_redaction};
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_read_thermal_zone_temp() {
        // Create a temporary directory.
        let dir = tempdir().unwrap();

        // Create a "thermal_zone0" directory inside the temporary directory.
        let thermal_zone_dir = dir.path().join("thermal_zone0");
        std::fs::create_dir(&thermal_zone_dir).unwrap();

        // Create a "temp" file inside the "thermal_zone0" directory.
        let temp_file_path = thermal_zone_dir.join("temp");
        let mut temp_file = File::create(temp_file_path).unwrap();

        // Write the temperature (in millidegrees Celsius) to the "temp" file.
        writeln!(temp_file, "50000").unwrap();

        // Create a "temp" file inside the "thermal_zone0" directory.
        let type_file_path = thermal_zone_dir.join("type");
        let mut type_file = File::create(type_file_path).unwrap();

        // Write the temperature (in millidegrees Celsius) to the "temp" file.
        writeln!(type_file, "cpu-temp").unwrap();

        // Call the function and check the result.
        let result = ThermalMetricsCollector::read_thermal_zone_temp(
            "thermal_zone0",
            dir.path().to_str().unwrap(),
        )
        .unwrap();

        assert_json_snapshot!(result, {".value.**.timestamp" => "[timestamp]", ".value.**.value" => rounded_redaction(5)});

        // Delete the temporary directory.
        dir.close().unwrap();
    }
}
