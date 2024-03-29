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

mod metric_report;
pub use metric_report::MetricReport;
pub use metric_report::MetricReportType;

mod metric_report_manager;
pub use metric_report_manager::MetricReportManager;

mod metric_reading;
pub use metric_reading::KeyedMetricReading;
pub use metric_reading::MetricReading;

mod timeseries;

mod metric_value;
pub use metric_value::MetricValue;

pub type MetricTimestamp = DateTime<Utc>;

mod crashfree_interval;
pub use crashfree_interval::CrashFreeIntervalTracker;

mod session_name;
pub use session_name::SessionName;

mod session_event_handler;
pub use session_event_handler::SessionEventHandler;
