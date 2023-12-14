//
// Copyright (c) Memfault, Inc.
// See License.txt for details
mod battery_reading_handler;
pub use battery_reading_handler::BatteryReadingHandler;

mod battery_monitor;
pub use battery_monitor::BatteryMonitor;
pub use battery_monitor::BatteryMonitorReading;
pub use battery_monitor::METRIC_BATTERY_SOC_PCT;
