//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};
use log::warn;
use ssf::{Handler, MsgMailbox, Service};

use std::time::Duration;
use std::{ops::Sub, thread::spawn};
use std::{process::Command, time::Instant};
use std::{str::FromStr, thread::sleep};

use eyre::{eyre, ErrReport, Result};

use crate::{
    config::Config,
    metrics::{
        battery::messages::BatteryReadingMessage,
        core_metrics::{
            METRIC_BATTERY_DISCHARGE_DURATION_MS, METRIC_BATTERY_SOC_PCT,
            METRIC_BATTERY_SOC_PCT_DROP, METRIC_BATTERY_SOH_PCT,
        },
        KeyedMetricReading, MetricReading, MetricsMBox, SysfsBatteryParser,
    },
};
use crate::{metrics::find_sysfs_battery_entry, util::time_measure::TimeMeasure};

const SYSFS_POWER_SUPPLY_DIR: &str = "/sys/class/power_supply";

// These states are based off the valid values for
// sys/class/power_supply/<supply_name>/status
// Read more here:
// https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-class-power
#[derive(Debug, PartialEq, Eq)]
pub enum ChargingState {
    Charging,
    Discharging,
    Full,
    Unknown,
    NotCharging,
    Invalid,
}

impl From<&str> for ChargingState {
    fn from(value: &str) -> Self {
        match value.trim() {
            "Charging" => ChargingState::Charging,
            "Discharging" => ChargingState::Discharging,
            "Full" => ChargingState::Full,
            "Not charging" => ChargingState::NotCharging,
            "Unknown" => ChargingState::Unknown,
            _ => ChargingState::Invalid,
        }
    }
}

// A single reading that describes
// the state of the device's battery
#[derive(Debug, PartialEq)]
pub struct BatteryMonitorReading {
    battery_soc_pct: f64,
    battery_charging_state: ChargingState,
    battery_soh_pct: Option<f64>,
}

impl BatteryMonitorReading {
    pub fn new(
        battery_soc_pct: f64,
        battery_charging_state: ChargingState,
        battery_soh_pct: Option<f64>,
    ) -> BatteryMonitorReading {
        BatteryMonitorReading {
            battery_soc_pct,
            battery_charging_state,
            battery_soh_pct,
        }
    }

    pub fn from_command(mut battery_info_command: Command) -> Result<Self> {
        let battery_info_output = battery_info_command.output()?;
        if !battery_info_output.status.success() {
            Err(eyre!(
                "Failed to execute {}. Battery percentage was not captured.",
                battery_info_command.get_program().to_string_lossy()
            ))
        } else {
            let output_string = String::from_utf8(battery_info_output.stdout)?;
            let battery_monitor_reading = BatteryMonitorReading::from_str(&output_string)?;
            Ok(battery_monitor_reading)
        }
    }
}

impl FromStr for BatteryMonitorReading {
    type Err = ErrReport;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((state_str, pct_str)) = s.trim().split_once(':') {
            let pct = pct_str.parse::<f64>();
            let charging_state = ChargingState::from(state_str);
            match (charging_state, pct) {
                (ChargingState::Invalid, _) => Err(eyre!("Invalid charging state: {}", state_str)),
                (_, Err(e)) => Err(eyre!("Couldn't parse battery percentage: {}", e)),
                (charging_state, Ok(p)) => {
                    if (0.0..=100.0).contains(&p) {
                        Ok(BatteryMonitorReading::new(p, charging_state, None))
                    } else {
                        Err(eyre!(
                            "Battery SOC percentage value {} is not in the range [0.0, 100.0]!",
                            p
                        ))
                    }
                }
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
    metrics_mbox: MetricsMBox,
}

impl<T> BatteryMonitor<T>
where
    T: TimeMeasure + Copy + Ord + Sub<T, Output = Duration>,
{
    pub fn new(metrics_mbox: MetricsMBox) -> Self {
        Self {
            previous_reading: None,
            last_reading_time: T::now(),
            metrics_mbox,
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
        let metrics = match (
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
                    ..
                }),
            ) => {
                let soc_pct = battery_monitor_reading.battery_soc_pct;
                let soc_pct_discharged =
                    (previous_soc_pct - battery_monitor_reading.battery_soc_pct).max(0.0);

                let mut metrics = vec![
                    KeyedMetricReading::add_to_counter(
                        METRIC_BATTERY_DISCHARGE_DURATION_MS.into(),
                        reading_duration.as_millis() as f64,
                    ),
                    KeyedMetricReading::add_to_counter(
                        METRIC_BATTERY_SOC_PCT_DROP.into(),
                        soc_pct_discharged,
                    ),
                    KeyedMetricReading::new(
                        METRIC_BATTERY_SOC_PCT.into(),
                        MetricReading::TimeWeightedAverage {
                            value: soc_pct,
                            timestamp: wall_time,
                            interval: chrono::Duration::from_std(reading_duration)?,
                        },
                    ),
                ];
                if let Some(soh_pct) = battery_monitor_reading.battery_soh_pct {
                    metrics.push(KeyedMetricReading::new(
                        METRIC_BATTERY_SOH_PCT.into(),
                        MetricReading::TimeWeightedAverage {
                            value: soh_pct,
                            timestamp: wall_time,
                            interval: chrono::Duration::from_std(reading_duration)?,
                        },
                    ));
                }
                metrics
            }
            // In all other cases only update the SoC percent
            _ => {
                let soc_pct = battery_monitor_reading.battery_soc_pct;

                let mut metrics = vec![
                    // Add 0.0 to these counters so if the device is charging
                    // for the full heartbeat duration these metrics are still
                    // populated
                    KeyedMetricReading::add_to_counter(
                        METRIC_BATTERY_DISCHARGE_DURATION_MS.into(),
                        0.0,
                    ),
                    KeyedMetricReading::add_to_counter(METRIC_BATTERY_SOC_PCT_DROP.into(), 0.0),
                    KeyedMetricReading::new(
                        METRIC_BATTERY_SOC_PCT.into(),
                        MetricReading::TimeWeightedAverage {
                            value: soc_pct,
                            timestamp: wall_time,
                            interval: chrono::Duration::from_std(reading_duration)?,
                        },
                    ),
                ];
                if let Some(soh_pct) = battery_monitor_reading.battery_soh_pct {
                    metrics.push(KeyedMetricReading::new(
                        METRIC_BATTERY_SOH_PCT.into(),
                        MetricReading::TimeWeightedAverage {
                            value: soh_pct,
                            timestamp: wall_time,
                            interval: chrono::Duration::from_std(reading_duration)?,
                        },
                    ));
                }
                metrics
            }
        };

        self.metrics_mbox.send_and_forget(metrics)?;
        self.previous_reading = Some(battery_monitor_reading);
        self.last_reading_time = reading_time;

        Ok(())
    }

    pub fn add_new_reading(
        &mut self,
        battery_monitor_reading: BatteryMonitorReading,
    ) -> Result<()> {
        self.update_metrics(battery_monitor_reading, T::now(), Utc::now())?;
        Ok(())
    }
}

impl<T> Service for BatteryMonitor<T>
where
    T: TimeMeasure + Copy + Ord + Sub<T, Output = Duration>,
{
    fn name(&self) -> &str {
        "BatteryMonitor"
    }
}

impl<T> Handler<BatteryReadingMessage> for BatteryMonitor<T>
where
    T: TimeMeasure + Copy + Ord + Sub<T, Output = Duration>,
{
    fn deliver(
        &mut self,
        m: BatteryReadingMessage,
    ) -> <BatteryReadingMessage as ssf::Message>::Reply {
        if let Err(e) = self.add_new_reading(m.reading) {
            warn!("Failed to add battery reading: {e}");
        }
    }
}

pub fn start_battery_reading_thread(
    config: &Config,
    battery_msg_mailbox: MsgMailbox<BatteryReadingMessage>,
) -> Result<()> {
    let battery_monitor_interval = config.battery_monitor_interval();
    let battery_info_command = config.battery_monitor_battery_info_command();
    let battery_info_command_string = battery_info_command.map(|command| command.to_string());
    let auto_mode = config.battery_monitor_auto_mode();
    let sysfs_parser = auto_mode
        .then(|| find_sysfs_battery_entry(SYSFS_POWER_SUPPLY_DIR))
        .and_then(|res| res.ok().flatten())
        .map(|path| SysfsBatteryParser::new(&path));

    // Do not spawn thread if auto mode disabled and no battery command
    if sysfs_parser.is_none() && battery_info_command_string.is_none() {
        return Ok(());
    }

    spawn(move || {
        let mut next_battery_interval = Instant::now() + battery_monitor_interval;
        loop {
            while Instant::now() < next_battery_interval {
                sleep(next_battery_interval - Instant::now());
            }
            next_battery_interval += battery_monitor_interval;
            if let Some(command_string) = &battery_info_command_string {
                let battery_info_command = Command::new(command_string);
                match BatteryMonitorReading::from_command(battery_info_command) {
                    Ok(reading) => {
                        if let Err(e) =
                            battery_msg_mailbox.send_and_forget(BatteryReadingMessage::new(reading))
                        {
                            warn!("Error updating battery monitor metrics: {}", e);
                        }
                    }
                    Err(e) => warn!("Failed to get battery reading: {e}"),
                }
            } else if let Some(parser) = &sysfs_parser {
                if let Err(e) = parser.reading().map(|reading| {
                    battery_msg_mailbox.send_and_forget(BatteryReadingMessage::new(reading))
                }) {
                    warn!("Failed to get battery reading: {e}")
                }
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::metrics::{MetricValue, TakeMetrics};
    use crate::test_utils::TestInstant;
    use rstest::rstest;
    use ssf::ServiceMock;

    #[rstest]
    #[case("Charging:80", true)]
    #[case("Discharging:80", true)]
    #[case("Not charging:80", true)]
    #[case("Isn't charging:80", false)]
    #[case("Charging:EIGHTY", false)]
    #[case("Charging:42.5", true)]
    #[case("Charging:42.five", false)]
    #[case("Charging:42.3.5", false)]
    #[case("Charging:-1", false)]
    #[case("Discharging:100.1", false)]
    #[case("Discharging:100.0", true)]
    #[case("Discharging:0.0", true)]
    #[case("Discharging:-0.1", false)]
    #[case("Full:100.0", true)]
    #[case("Unknown:80", true)]
    fn test_parse(#[case] cmd_output: &str, #[case] is_ok: bool) {
        assert_eq!(BatteryMonitorReading::from_str(cmd_output).is_ok(), is_ok);
    }

    #[rstest]
    // Single reading results in expected metrics
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Charging, None)], 30, 90.0, 0.0, 0.0)]
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Charging, None), BatteryMonitorReading::new(100.0, ChargingState::Charging, None)], 30, 95.0, 0.0, 0.0)]
    // Battery discharges between readings
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging, None), BatteryMonitorReading::new(85.0, ChargingState::Discharging, None)], 30, 87.5, 5.0, 30000.0)]
    // Battery is discharging, then charging, then discharging again
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(85.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(90.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(90.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging, None)],
           30,
           87.0,
           15.0,
           60000.0)]
    // Continuous discharge
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(70.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(60.0, ChargingState::Discharging, None)],
           30,
           75.0,
           30.0,
           90000.0)]
    // Continuous charge
    #[case(vec![BatteryMonitorReading::new(60.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(70.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(90.0, ChargingState::Charging, None)],
           30,
           75.0,
           0.0,
           0.0)]
    // Battery was charged in between monitoring calls
    #[case(vec![BatteryMonitorReading::new(60.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging, None),],
           30,
           70.0,
           0.0,
           30000.0)]
    // Discharge then charge to full
    #[case(vec![BatteryMonitorReading::new(90.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(70.0, ChargingState::Discharging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(100.0, ChargingState::Full, None)],
           30,
           84.0,
           20.0,
           60000.0)]
    // Check unknown and not charging states
    #[case(vec![BatteryMonitorReading::new(60.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(70.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(80.0, ChargingState::Unknown, None),
                BatteryMonitorReading::new(90.0, ChargingState::NotCharging, None)],
           30,
           75.0,
           0.0,
           0.0)]
    // Check measurements with 0 seconds between
    #[case(vec![BatteryMonitorReading::new(80.0, ChargingState::Charging, None),
                BatteryMonitorReading::new(85.0, ChargingState::NotCharging, None),
                BatteryMonitorReading::new(90.0, ChargingState::NotCharging, None)],
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
        let mut metrics_mock = ServiceMock::new();
        let mut battery_monitor = BatteryMonitor {
            metrics_mbox: metrics_mock.mbox.clone(),
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
        let metrics = metrics_mock.take_metrics().unwrap();
        let soc_pct_key = METRIC_BATTERY_SOC_PCT.into();

        match metrics.get(&soc_pct_key).unwrap() {
            MetricValue::Number(e) => {
                if expected_soc_pct.is_finite() {
                    assert_eq!(*e, expected_soc_pct);
                } else {
                    assert!(e.is_nan());
                }
            }
            _ => panic!("This test only expects number metric values!"),
        }

        let soc_pct_discharge_key = METRIC_BATTERY_SOC_PCT_DROP.into();
        match metrics.get(&soc_pct_discharge_key).unwrap() {
            MetricValue::Number(e) => assert_eq!(*e, expected_soc_pct_discharge),
            _ => panic!("This test only expects number metric values!"),
        }

        let soc_discharge_duration_key = METRIC_BATTERY_DISCHARGE_DURATION_MS.into();
        match metrics.get(&soc_discharge_duration_key).unwrap() {
            MetricValue::Number(e) => assert_eq!(*e, expected_discharge_duration),
            _ => panic!("This test only expects number metric values!"),
        }
    }

    #[rstest]
    #[case("Charging", ChargingState::Charging)]
    #[case("Discharging", ChargingState::Discharging)]
    #[case("Full", ChargingState::Full)]
    #[case("Not charging", ChargingState::NotCharging)]
    #[case("Unknown", ChargingState::Unknown)]
    #[case("InvalidState", ChargingState::Invalid)]
    #[case("  Charging  ", ChargingState::Charging)]
    fn test_charging_state_parsing(#[case] raw_str: &str, #[case] expected: ChargingState) {
        let parsed_state = ChargingState::from(raw_str);
        assert_eq!(parsed_state, expected);
    }
}
