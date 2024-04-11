//
// Copyright (c) Memfault, Inc.
// See License.txt for details
// Connectivity metrics
pub const METRIC_MF_SYNC_SUCCESS: &str = "sync_memfault_successful";
pub const METRIC_MF_SYNC_FAILURE: &str = "sync_memfault_failure";
pub const METRIC_CONNECTED_TIME: &str = "connectivity_connected_time_ms";
pub const METRIC_EXPECTED_CONNECTED_TIME: &str = "connectivity_expected_time_ms";
pub const METRIC_SYNC_SUCCESS: &str = "sync_successful";
pub const METRIC_SYNC_FAILURE: &str = "sync_failure";

// Stability metrics
pub const METRIC_OPERATIONAL_HOURS: &str = "operational_hours";
pub const METRIC_OPERATIONAL_CRASHFREE_HOURS: &str = "operational_crashfree_hours";
pub const METRIC_OPERATIONAL_CRASHES: &str = "operational_crashes";

// Battery metrics
pub const METRIC_BATTERY_DISCHARGE_DURATION_MS: &str = "battery_discharge_duration_ms";
pub const METRIC_BATTERY_SOC_PCT_DROP: &str = "battery_soc_pct_drop";
pub const METRIC_BATTERY_SOC_PCT: &str = "battery_soc_pct";
