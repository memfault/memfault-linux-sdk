//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};

use std::ops::Sub;
use std::process::Command;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eyre::{eyre, ErrReport, Result};

use crate::metrics::{HeartbeatManager, KeyedMetricReading, MetricReading, MetricStringKey};
use crate::util::time_measure::TimeMeasure;

const METRIC_BATTERY_DISCHARGE_DURATION_MS: &str = "battery_discharge_duration_ms";
const METRIC_BATTERY_SOC_PCT_DROP: &str = "battery_soc_pct_drop";
pub const METRIC_BATTERY_SOC_PCT: &str = "battery_soc_pct";

// These states are based off the valid values for
// sys/class/power_supply/<supply_name>/status
// Read more here:
// https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-class-power
enum ChargingState {
    Charging,
    Discharging,
    Full,
    Unknown,
    NotCharging,
    Invalid,
}

// A single reading that describes
// the state of the device's battery
pub struct BatteryMonitorReading {
    battery_soc_pct: f64,
    battery_charging_state: ChargingState,
}

impl BatteryMonitorReading {
    fn new(battery_soc_pct: f64, battery_charging_state: ChargingState) -> BatteryMonitorReading {
        BatteryMonitorReading {
            battery_soc_pct,
            battery_charging_state,
        }
    }
}

impl FromStr for BatteryMonitorReading {
    type Err = ErrReport;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((state_str, pct_str)) = s.trim().split_once(':') {
            let charging_state = match state_str {
                "Charging" => ChargingState::Charging,
                "Discharging" => ChargingState::Discharging,
                "Full" => ChargingState::Full,
                "Not charging" => ChargingState::NotCharging,
                "Unknown" => ChargingState::Unknown,
                _ => ChargingState::Invalid,
            };
            let pct = pct_str.parse::<f64>();
            match (charging_state, pct) {
                (ChargingState::Invalid, _) => Err(eyre!("Invalid charging state: {}", state_str)),
                (_, Err(e)) => Err(eyre!("Couldn't parse battery percentage: {}", e)),
                (charging_state, Ok(p)) => Ok(BatteryMonitorReading::new(p, charging_state)),
            }
        } else {
            Err(eyre!(
                "Invalid output from command configured via `battery_percentage_command`"
            ))
        }
    }
}

// Since some of the battery metrics recorded
// are calculated based on both the current and previous reading
// (such as battery_soc_pct_drop), this struct needs to
// store the previous battery percentage as well as when that
// perecentage was recorded.
pub struct BatteryMonitor<T: TimeMeasure> {
    previous_reading: Option<BatteryMonitorReading>,
    last_reading_time: T,
    heartbeat_manager: Arc<Mutex<HeartbeatManager>>,
}

impl<T> BatteryMonitor<T>
where
    T: TimeMeasure + Copy + Ord + Sub<T, Output = Duration>,
{
    pub fn new(heartbeat_manager: Arc<Mutex<HeartbeatManager>>) -> Self {
        Self {
            previous_reading: None,
            last_reading_time: T::now(),
            heartbeat_manager,
        }
    }

    // Writes new values for battery_discharge_duration_ms,  battery_soc_pct_drop,
    // and battery_soc_pct to the in memory metric store
    fn update_metrics(
        &mut self,
        battery_monitor_reading: BatteryMonitorReading,
        reading_time: T,
        wall_time: DateTime<Utc>,
    ) -> Result<()> {
        let reading_duration = reading_time.since(&self.last_reading_time);
        match (
            &battery_monitor_reading.battery_charging_state,
            &self.previous_reading,
        ) {
            // Update battery discharge metrics only when there is a previous
            // reading and both the previous AND current
            // charging state are Discharging
            (
                ChargingState::Discharging,
                Some(BatteryMonitorReading {
                    battery_soc_pct: previous_soc_pct,
                    battery_charging_state: ChargingState::Discharging,
                }),
            ) => {
                let soc_pct = battery_monitor_reading.battery_soc_pct;
                let soc_pct_discharged =
                    (previous_soc_pct - battery_monitor_reading.battery_soc_pct).max(0.0);

                let mut heartbeat_manager = self.heartbeat_manager.lock().expect("Mutex Poisoned");
                heartbeat_manager.add_to_counter(
                    METRIC_BATTERY_DISCHARGE_DURATION_MS,
                    reading_duration.as_millis() as f64,
                )?;

                heartbeat_manager
                    .add_to_counter(METRIC_BATTERY_SOC_PCT_DROP, soc_pct_discharged)?;

                let battery_soc_pct_key = MetricStringKey::from_str(METRIC_BATTERY_SOC_PCT)
                    .unwrap_or_else(|_| panic!("Invalid metric name: {}", METRIC_BATTERY_SOC_PCT));
                heartbeat_manager.add_metric(KeyedMetricReading::new(
                    battery_soc_pct_key,
                    MetricReading::Gauge {
                        value: soc_pct,
                        timestamp: wall_time,
                        interval: chrono::Duration::from_std(reading_duration)?,
                    },
                ))?;
            }
            // In all other cases only update the SoC percent
            _ => {
                let soc_pct = battery_monitor_reading.battery_soc_pct;

                let mut heartbeat_manager = self.heartbeat_manager.lock().expect("Mutex Poisoned");

                // Add 0.0 to these counters so if the device is charging
                // for the full heartbeat duration these metrics are still
                // populated
                heartbeat_manager.add_to_counter(METRIC_BATTERY_DISCHARGE_DURATION_MS, 0.0)?;
                heartbeat_manager.add_to_counter(METRIC_BATTERY_SOC_PCT_DROP, 0.0)?;

                let battery_soc_pct_key = MetricStringKey::from_str(METRIC_BATTERY_SOC_PCT)
                    .unwrap_or_else(|_| panic!("Invalid metric name: {}", METRIC_BATTERY_SOC_PCT));
                heartbeat_manager.add_metric(KeyedMetricReading::new(
                    battery_soc_pct_key,
                    MetricReading::Gauge {
                        value: soc_pct,
                        timestamp: wall_time,
                        interval: chrono::Duration::from_std(reading_duration)?,
                    },
                ))?;
            }
        }

        self.previous_reading = Some(battery_monitor_reading);
        self.last_reading_time = reading_time;

        Ok(())
    }

    pub fn update_via_command(&mut self, mut battery_info_command: Command) -> Result<()> {
        let battery_info_output = battery_info_command.output()?;
        if !battery_info_output.status.success() {
            Err(eyre!(
                "Failed to execute {}. Battery percentage was not captured.",
                battery_info_command.get_program().to_string_lossy()
            ))
        } else {
            let output_string = String::from_utf8(battery_info_output.stdout)?;
            let battery_monitor_reading = BatteryMonitorReading::from_str(&output_string)?;
            self.add_new_reading(battery_monitor_reading)?;
            Ok(())
        }
    }

    pub fn add_new_reading(
        &mut self,
        battery_monitor_reading: BatteryMonitorReading,
    ) -> Result<()> {
        self.update_metrics(battery_monitor_reading, T::now(), Utc::now())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::metrics::MetricValue;
    use crate::test_utils::TestInstant;
    use rstest::rstest;

    #[rstest]
    #[case("Charging:80", true)]
    #[case("Discharging:80", true)]
    #[case("Not charging:80", true)]
    #[case("Isn't charging:80", false)]
    #[case("Charging:EIGHTY", false)]
    #[case("Charging:42.5", true)]
    #[case("Charging:42.five", false)]
    #[case("Charging:42.3.5", false)]
    #[case("Full:100.0", true)]
    #[case("Unknown:80", true)]
    fn test_parse(#[case] cmd_output: &str, #[case] is_ok: bool) {
        assert_eq!(BatteryMonitorReading::from_str(cmd_output).is_ok(), is_ok);
    }

    #[rstest]
    // Single reading results in expected metrics
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Charging)], 30, 90.0, 0.0, 0.0)]
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Charging), BatteryMonitorReading::new(100.0, ChargingState::Charging)], 30, 95.0, 0.0, 0.0)]
    // Battery discharges between readings
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging), BatteryMonitorReading::new(85.0, ChargingState::Discharging)], 30, 87.5, 5.0, 30000.0)]
    // Battery is discharging, then charging, then discharging again
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging),
                BatteryMonitorReading::new(85.0, ChargingState::Discharging),
                BatteryMonitorReading::new(90.0, ChargingState::Charging),
                BatteryMonitorReading::new(90.0, ChargingState::Discharging),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging)],
           30,
           87.0,
           15.0,
           60000.0)]
    // Continous discharge
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging),
                BatteryMonitorReading::new(70.0, ChargingState::Discharging),
                BatteryMonitorReading::new(60.0, ChargingState::Discharging)],
           30,
           75.0,
           30.0,
           90000.0)]
    // Continous charge
    #[case(vec![BatteryMonitorReading::new(60.0, ChargingState::Charging),
                BatteryMonitorReading::new(70.0, ChargingState::Charging),
                BatteryMonitorReading::new(80.0, ChargingState::Charging),
                BatteryMonitorReading::new(90.0, ChargingState::Charging)],
           30,
           75.0,
           0.0,
           0.0)]
    // Battery was charged in between monitoring calls
    #[case(vec![BatteryMonitorReading::new(60.0, ChargingState::Discharging),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging),],
           30,
           70.0,
           0.0,
           30000.0)]
    // Discharge then charge to full
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging),
                BatteryMonitorReading::new(70.0, ChargingState::Discharging),
                BatteryMonitorReading::new(80.0, ChargingState::Charging),
                BatteryMonitorReading::new(100.0, ChargingState::Full)],
           30,
           84.0,
           20.0,
           60000.0)]
    // Check unknown and not charging states
    #[case(vec![BatteryMonitorReading::new(60.0, ChargingState::Charging),
                BatteryMonitorReading::new(70.0, ChargingState::Charging),
                BatteryMonitorReading::new(80.0, ChargingState::Unknown),
                BatteryMonitorReading::new(90.0, ChargingState::NotCharging)],
           30,
           75.0,
           0.0,
           0.0)]
    // Check measurements with 0 seconds between
    #[case(vec![BatteryMonitorReading::new(80.0, ChargingState::Charging),
                BatteryMonitorReading::new(85.0, ChargingState::NotCharging),
                BatteryMonitorReading::new(90.0, ChargingState::NotCharging)],
           0,
           f64::NAN,
           0.0,
           0.0)]
    fn test_update_metrics_soc_pct(
        #[case] battery_monitor_readings: Vec<BatteryMonitorReading>,
        #[case] seconds_between_readings: u64,
        #[case] expected_soc_pct: f64,
        #[case] expected_soc_pct_discharge: f64,
        #[case] expected_discharge_duration: f64,
    ) {
        let now = TestInstant::now();
        let heartbeat_manager = Arc::new(Mutex::new(HeartbeatManager::new()));
        let mut battery_monitor = BatteryMonitor {
            heartbeat_manager,
            last_reading_time: now,
            previous_reading: None,
        };

        let mut ts = Utc::now();
        for reading in battery_monitor_readings {
            TestInstant::sleep(Duration::from_secs(seconds_between_readings));
            ts += chrono::Duration::seconds(seconds_between_readings as i64);
            battery_monitor
                .update_metrics(reading, TestInstant::now(), ts)
                .unwrap();
        }
        let metrics = battery_monitor
            .heartbeat_manager
            .lock()
            .unwrap()
            .take_metrics();
        let soc_pct_key = METRIC_BATTERY_SOC_PCT.parse::<MetricStringKey>().unwrap();

        match metrics.get(&soc_pct_key).unwrap() {
            MetricValue::Number(e) => {
                if expected_soc_pct.is_finite() {
                    assert_eq!(*e, expected_soc_pct);
                } else {
                    assert!(e.is_nan());
                }
            }
        }

        let soc_pct_discharge_key = METRIC_BATTERY_SOC_PCT_DROP
            .parse::<MetricStringKey>()
            .unwrap();
        match metrics.get(&soc_pct_discharge_key).unwrap() {
            MetricValue::Number(e) => assert_eq!(*e, expected_soc_pct_discharge),
        }

        let soc_discharge_duration_key = METRIC_BATTERY_DISCHARGE_DURATION_MS
            .parse::<MetricStringKey>()
            .unwrap();
        match metrics.get(&soc_discharge_duration_key).unwrap() {
            MetricValue::Number(e) => assert_eq!(*e, expected_discharge_duration),
        }
    }
}
