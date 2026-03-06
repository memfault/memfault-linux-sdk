//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::{read_dir, read_to_string},
    path::{Path, PathBuf},
};

use eyre::{Report, Result};
use itertools::Itertools;

use crate::metrics::BatteryMonitorReading;

pub struct SysfsBatteryParser {
    path: PathBuf,
}

impl SysfsBatteryParser {
    pub fn new(sysfs_path: &Path) -> Self {
        Self {
            path: sysfs_path.into(),
        }
    }

    pub fn reading(&self) -> Result<BatteryMonitorReading> {
        let charge_state_path = self.path.join("status");
        let charge_state = read_to_string(charge_state_path)?.as_str().into();
        let charge_pct_path = self.path.join("capacity");
        let charge_pct_str = read_to_string(charge_pct_path)?.trim().to_string();
        let charge_pct: f64 = charge_pct_str.parse()?;

        // Calculate battery state of health (SoH) if the required sysfs files exist
        let battery_soh_pct = self.calculate_soh().ok();

        let bat_reading = BatteryMonitorReading::new(charge_pct, charge_state, battery_soh_pct);
        Ok(bat_reading)
    }

    fn calculate_soh(&self) -> Result<f64> {
        let energy_full_path = self.path.join("energy_full");
        let energy_full_str = read_to_string(energy_full_path)?.trim().to_string();
        let energy_full: f64 = energy_full_str.parse()?;

        let energy_full_design_path = self.path.join("energy_full_design");
        let energy_full_design_str = read_to_string(energy_full_design_path)?.trim().to_string();
        let energy_full_design: f64 = energy_full_design_str.parse()?;

        if energy_full_design > 0.0 {
            // limit to 100.0%, in case the battery is reporting a higher full
            // capacity than the design capacity
            Ok(((energy_full / energy_full_design) * 100.0).min(100.0))
        } else {
            Err(eyre::eyre!("energy_full_design is zero or negative"))
        }
    }
}

/// Sysfs power supply type
///
/// Definition pulled from here: https://elixir.bootlin.com/linux/v6.13/source/include/linux/power_supply.h#L179
#[derive(Debug, PartialEq, Eq)]
enum PowerSupplyType {
    Unknown,
    Battery,
    Ups,
    Mains,
    Usb,
    UsbDcp,
    UsbCdp,
    UsbAca,
    UsbTypeC,
    UsbPd,
    UsbPdDrp,
    AppleBrickId,
    Wireless,
}

impl TryFrom<&str> for PowerSupplyType {
    type Error = Report;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "unknown" => Ok(PowerSupplyType::Unknown),
            "battery" => Ok(PowerSupplyType::Battery),
            "ups" => Ok(PowerSupplyType::Ups),
            "mains" => Ok(PowerSupplyType::Mains),
            "usb" => Ok(PowerSupplyType::Usb),
            "usb_dcp" => Ok(PowerSupplyType::UsbDcp),
            "usb_cdp" => Ok(PowerSupplyType::UsbCdp),
            "usb_aca" => Ok(PowerSupplyType::UsbAca),
            "usb_type_c" => Ok(PowerSupplyType::UsbTypeC),
            "usb_pd" => Ok(PowerSupplyType::UsbPd),
            "usb_pd_drp" => Ok(PowerSupplyType::UsbPdDrp),
            "apple_brick_id" => Ok(PowerSupplyType::AppleBrickId),
            "wireless" => Ok(PowerSupplyType::Wireless),
            invalid => Err(eyre::eyre!("Invalid power supply type: {}", invalid)),
        }
    }
}

/// Finds the battery entry in the power supply dir
///
/// Note that this will find the first battery entry and use it. Currently
/// we're not supporting mult-battery systems.
pub fn find_sysfs_battery_entry(sysfs_power_supply_dir: &str) -> Result<Option<PathBuf>> {
    let bat_dir = read_dir(sysfs_power_supply_dir)?
        .filter_map(|dir| Some(dir.ok()?.path()))
        .sorted()
        .filter_map(|bat_dir| {
            let type_dir = bat_dir.join("type");

            let type_string = read_to_string(&type_dir).ok()?;
            let ps_type = PowerSupplyType::try_from(type_string.trim()).ok()?;

            (ps_type == PowerSupplyType::Battery).then_some(bat_dir)
        })
        .next();

    Ok(bat_dir)
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use crate::metrics::battery::battery_monitor::ChargingState;

    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    #[case("unknown", PowerSupplyType::Unknown)]
    #[case("UNKNOWN", PowerSupplyType::Unknown)]
    #[case("Unknown", PowerSupplyType::Unknown)]
    #[case("battery", PowerSupplyType::Battery)]
    #[case("BATTERY", PowerSupplyType::Battery)]
    #[case("Battery", PowerSupplyType::Battery)]
    #[case("ups", PowerSupplyType::Ups)]
    #[case("UPS", PowerSupplyType::Ups)]
    #[case("mains", PowerSupplyType::Mains)]
    #[case("MAINS", PowerSupplyType::Mains)]
    #[case("usb", PowerSupplyType::Usb)]
    #[case("USB", PowerSupplyType::Usb)]
    #[case("usb_dcp", PowerSupplyType::UsbDcp)]
    #[case("USB_DCP", PowerSupplyType::UsbDcp)]
    #[case("usb_cdp", PowerSupplyType::UsbCdp)]
    #[case("USB_CDP", PowerSupplyType::UsbCdp)]
    #[case("usb_aca", PowerSupplyType::UsbAca)]
    #[case("USB_ACA", PowerSupplyType::UsbAca)]
    #[case("usb_type_c", PowerSupplyType::UsbTypeC)]
    #[case("USB_TYPE_C", PowerSupplyType::UsbTypeC)]
    #[case("usb_pd", PowerSupplyType::UsbPd)]
    #[case("USB_PD", PowerSupplyType::UsbPd)]
    #[case("usb_pd_drp", PowerSupplyType::UsbPdDrp)]
    #[case("USB_PD_DRP", PowerSupplyType::UsbPdDrp)]
    #[case("apple_brick_id", PowerSupplyType::AppleBrickId)]
    #[case("APPLE_BRICK_ID", PowerSupplyType::AppleBrickId)]
    #[case("wireless", PowerSupplyType::Wireless)]
    #[case("WIRELESS", PowerSupplyType::Wireless)]
    fn test_power_supply_type_from_valid_str(
        #[case] input: &str,
        #[case] expected: PowerSupplyType,
    ) {
        let result = PowerSupplyType::try_from(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[rstest]
    #[case("invalid")]
    #[case("power_source")]
    #[case("")]
    #[case("batteryX")]
    #[case("usb_invalid")]
    fn test_power_supply_type_from_invalid_str(#[case] input: &str) {
        let result = PowerSupplyType::try_from(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_find_sysfs_battery_entry_finds_battery() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let power_supply_dir = temp_dir.path();

        let usb_dir = power_supply_dir.join("AC");
        std::fs::create_dir(&usb_dir).expect("Failed to create AC dir");
        let mut usb_type_file =
            File::create(usb_dir.join("type")).expect("Failed to create type file");
        usb_type_file
            .write_all(b"Mains\n")
            .expect("Failed to write to type file");

        let bat_dir = power_supply_dir.join("BAT0");
        std::fs::create_dir(&bat_dir).expect("Failed to create BAT0 dir");
        let mut bat_type_file =
            File::create(bat_dir.join("type")).expect("Failed to create type file");
        bat_type_file
            .write_all(b"Battery\n")
            .expect("Failed to write to type file");

        let usb_pd_dir = power_supply_dir.join("USBC");
        std::fs::create_dir(&usb_pd_dir).expect("Failed to create USBC dir");
        let mut usb_pd_type_file =
            File::create(usb_pd_dir.join("type")).expect("Failed to create type file");
        usb_pd_type_file
            .write_all(b"USB_PD\n")
            .expect("Failed to write to type file");

        let result = find_sysfs_battery_entry(power_supply_dir.to_str().unwrap())
            .expect("Failed to find battery entry");

        assert!(result.is_some());
        let full_path = power_supply_dir.join("BAT0");
        assert_eq!(result.unwrap(), full_path);
    }

    #[test]
    fn test_find_sysfs_battery_entry_no_battery() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let power_supply_dir = temp_dir.path();

        let ac_dir = power_supply_dir.join("AC");
        std::fs::create_dir(&ac_dir).expect("Failed to create AC dir");
        let mut ac_type_file =
            File::create(ac_dir.join("type")).expect("Failed to create type file");
        ac_type_file
            .write_all(b"Mains\n")
            .expect("Failed to write to type file");

        let result = find_sysfs_battery_entry(power_supply_dir.to_str().unwrap())
            .expect("Failed to call find_sysfs_battery_entry");

        assert!(result.is_none());
    }

    #[test]
    fn test_multiple_batteries_sorted() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let power_supply_dir = temp_dir.path();

        let bat1_dir = power_supply_dir.join("BAT1");
        std::fs::create_dir(&bat1_dir).expect("Failed to create BAT1 dir");
        let mut bat1_type_file =
            File::create(bat1_dir.join("type")).expect("Failed to create type file");
        bat1_type_file
            .write_all(b"Battery\n")
            .expect("Failed to write to type file");

        let bat0_dir = power_supply_dir.join("BAT0");
        std::fs::create_dir(&bat0_dir).expect("Failed to create BAT0 dir");
        let mut bat0_type_file =
            File::create(bat0_dir.join("type")).expect("Failed to create type file");
        bat0_type_file
            .write_all(b"Battery\n")
            .expect("Failed to write to type file");

        let result = find_sysfs_battery_entry(power_supply_dir.to_str().unwrap())
            .expect("Failed to find battery entry");

        assert!(result.is_some());
        let full_path = power_supply_dir.join("BAT0");
        assert_eq!(result.unwrap(), full_path);
    }

    #[test]
    fn test_find_sysfs_battery_entry_invalid_type_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let power_supply_dir = temp_dir.path();

        let invalid_dir = power_supply_dir.join("INVALID");
        std::fs::create_dir(&invalid_dir).expect("Failed to create INVALID dir");
        let mut invalid_type_file =
            File::create(invalid_dir.join("type")).expect("Failed to create type file");
        invalid_type_file
            .write_all(b"InvalidType\n")
            .expect("Failed to write to type file");

        let bat_dir = power_supply_dir.join("BAT0");
        std::fs::create_dir(&bat_dir).expect("Failed to create BAT0 dir");
        let mut bat_type_file =
            File::create(bat_dir.join("type")).expect("Failed to create type file");
        bat_type_file
            .write_all(b"Battery\n")
            .expect("Failed to write to type file");

        let result = find_sysfs_battery_entry(power_supply_dir.to_str().unwrap())
            .expect("Failed to find battery entry");

        assert!(result.is_some());
        let full_path = power_supply_dir.join("BAT0");
        assert_eq!(result.unwrap(), full_path);
    }

    #[test]
    fn test_find_sysfs_battery_entry_empty_directory() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let power_supply_dir = temp_dir.path();

        let result = find_sysfs_battery_entry(power_supply_dir.to_str().unwrap())
            .expect("Failed to call find_sysfs_battery_entry");

        assert!(result.is_none());
    }

    #[rstest]
    #[case("75\n", "Charging\n", 75.0, ChargingState::Charging)]
    #[case("50", "Discharging", 50.0, ChargingState::Discharging)]
    #[case("100", "Full", 100.0, ChargingState::Full)]
    #[case("0", "Not charging", 0.0, ChargingState::NotCharging)]
    #[case("85", "Unknown", 85.0, ChargingState::Unknown)]
    #[case("60", "InvalidState", 60.0, ChargingState::Invalid)]
    fn test_sysfs_battery_parser_reading(
        #[case] input_pct: &str,
        #[case] input_state: &str,
        #[case] expected_pct: f64,
        #[case] expected_state: ChargingState,
    ) {
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let battery_dir = temp_dir.path();

        let mut status_file =
            File::create(battery_dir.join("status")).expect("Failed to create status file");
        status_file
            .write_all(input_state.as_bytes())
            .expect("Failed to write to status file");

        let mut capacity_file =
            File::create(battery_dir.join("capacity")).expect("Failed to create capacity file");
        capacity_file
            .write_all(input_pct.as_bytes())
            .expect("Failed to write to capacity file");

        let parser = SysfsBatteryParser::new(battery_dir);
        let result = parser.reading().unwrap();

        let expected_reading = BatteryMonitorReading::new(expected_pct, expected_state, None);
        assert_eq!(result, expected_reading);
    }

    #[test]
    fn test_sysfs_battery_parser_with_soh() {
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let battery_dir = temp_dir.path();

        let mut status_file =
            File::create(battery_dir.join("status")).expect("Failed to create status file");
        status_file
            .write_all(b"Discharging\n")
            .expect("Failed to write to status file");

        let mut capacity_file =
            File::create(battery_dir.join("capacity")).expect("Failed to create capacity file");
        capacity_file
            .write_all(b"75")
            .expect("Failed to write to capacity file");

        let mut energy_full_file = File::create(battery_dir.join("energy_full"))
            .expect("Failed to create energy_full file");
        energy_full_file
            .write_all(b"29550")
            .expect("Failed to write to energy_full file");

        let mut energy_full_design_file = File::create(battery_dir.join("energy_full_design"))
            .expect("Failed to create energy_full_design file");
        energy_full_design_file
            .write_all(b"35000")
            .expect("Failed to write to energy_full_design file");

        let parser = SysfsBatteryParser::new(battery_dir);
        let result = parser.reading().unwrap();

        // Expected SoH: (29550 / 35000) * 100 = 84.42857...
        let expected_soh = (29550.0 / 35000.0) * 100.0;
        let expected_reading =
            BatteryMonitorReading::new(75.0, ChargingState::Discharging, Some(expected_soh));
        assert_eq!(result, expected_reading);
    }

    #[test]
    fn test_sysfs_battery_parser_soh_missing_files() {
        use std::fs::File;
        use std::io::Write;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let battery_dir = temp_dir.path();

        let mut status_file =
            File::create(battery_dir.join("status")).expect("Failed to create status file");
        status_file
            .write_all(b"Charging\n")
            .expect("Failed to write to status file");

        let mut capacity_file =
            File::create(battery_dir.join("capacity")).expect("Failed to create capacity file");
        capacity_file
            .write_all(b"80")
            .expect("Failed to write to capacity file");

        // Don't create energy_full or energy_full_design files

        let parser = SysfsBatteryParser::new(battery_dir);
        let result = parser.reading().unwrap();

        // Should succeed but with None for SoH
        let expected_reading = BatteryMonitorReading::new(80.0, ChargingState::Charging, None);
        assert_eq!(result, expected_reading);
    }
}
