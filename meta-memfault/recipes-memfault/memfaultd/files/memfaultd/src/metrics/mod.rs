//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};

mod battery;
pub use battery::BatteryMonitor;
pub use battery::BatteryMonitorReading;
pub use battery::BatteryReadingHandler;

mod connectivity;
pub use connectivity::ConnectivityMonitor;
pub use connectivity::ReportSyncEventHandler;

mod metric_string_key;
pub use metric_string_key::MetricStringKey;

mod heartbeat;
pub use heartbeat::HeartbeatManager;

mod metric_reading;
pub use metric_reading::KeyedMetricReading;
pub use metric_reading::MetricReading;

mod metric_value;
pub use metric_value::MetricValue;

pub type MetricTimestamp = DateTime<Utc>;

mod crashfree_interval;
pub use crashfree_interval::CrashFreeIntervalTracker;
