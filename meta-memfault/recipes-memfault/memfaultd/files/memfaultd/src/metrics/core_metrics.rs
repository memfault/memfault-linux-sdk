//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::HashSet;

use super::MetricStringKey;
use crate::util::wildcard_pattern::WildcardPattern;

// Connectivity metrics
pub const METRIC_MF_SYNC_SUCCESS: &str = "sync_memfault_successful";
pub const METRIC_MF_SYNC_FAILURE: &str = "sync_memfault_failure";
pub const METRIC_CONNECTED_TIME: &str = "connectivity_connected_time_ms";
pub const METRIC_EXPECTED_CONNECTED_TIME: &str = "connectivity_expected_time_ms";
pub const METRIC_SYNC_SUCCESS: &str = "sync_successful";
pub const METRIC_SYNC_FAILURE: &str = "sync_failure";
pub const METRIC_CONNECTIVITY_RECV_BYTES: &str = "connectivity_recv_bytes";
pub const METRIC_CONNECTIVITY_SENT_BYTES: &str = "connectivity_sent_bytes";
pub const METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_PREFIX: &str = "connectivity_";
pub const METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_SUFFIX: &str = "_recv_bytes";
pub const METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_PREFIX: &str = "connectivity_";
pub const METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_SUFFIX: &str = "_sent_bytes";

// Stability metrics
pub const METRIC_OPERATIONAL_HOURS: &str = "operational_hours";
pub const METRIC_OPERATIONAL_CRASHFREE_HOURS: &str = "operational_crashfree_hours";
pub const METRIC_OPERATIONAL_CRASHES: &str = "operational_crashes";
pub const METRIC_OPERATIONAL_CRASHES_PROCESS_PREFIX: &str = "operational_crashes_";

// Battery metrics
pub const METRIC_BATTERY_DISCHARGE_DURATION_MS: &str = "battery_discharge_duration_ms";
pub const METRIC_BATTERY_SOC_PCT_DROP: &str = "battery_soc_pct_drop";
pub const METRIC_BATTERY_SOC_PCT: &str = "battery_soc_pct";

// Memory Metrics
pub const METRIC_MEMORY_PCT: &str = "memory_pct";

// CPU Metrics
pub const METRIC_CPU_USAGE_PCT: &str = "cpu_usage_pct";

// Disk Space Metrics
pub const METRIC_STORAGE_USED_DISK_PCT_PREFIX: &str = "storage_used_";
pub const METRIC_STORAGE_USED_DISK_PCT_SUFFIX: &str = "_pct";

// Process Metrics
// Note: The metric keys for per-process core metrics follow the below
// format:
//     "<PREFIX><PROCESS NAME><SUFFIX>"
pub const METRIC_CPU_USAGE_PROCESS_PCT_PREFIX: &str = "cpu_usage_";
pub const METRIC_CPU_USAGE_PROCESS_PCT_SUFFIX: &str = "_pct";
pub const METRIC_MEMORY_PROCESS_PCT_PREFIX: &str = "memory_";
pub const METRIC_MEMORY_PROCESS_PCT_SUFFIX: &str = "_pct";

const SESSION_CORE_METRICS: &[&str; 12] = &[
    METRIC_MF_SYNC_FAILURE,
    METRIC_MF_SYNC_SUCCESS,
    METRIC_BATTERY_DISCHARGE_DURATION_MS,
    METRIC_BATTERY_SOC_PCT_DROP,
    METRIC_CONNECTED_TIME,
    METRIC_EXPECTED_CONNECTED_TIME,
    METRIC_SYNC_FAILURE,
    METRIC_SYNC_SUCCESS,
    METRIC_OPERATIONAL_CRASHES,
    METRIC_MEMORY_PCT,
    METRIC_CONNECTIVITY_RECV_BYTES,
    METRIC_CONNECTIVITY_SENT_BYTES,
];

const SESSION_CORE_WILDCARD_METIRCS: &[(&str, &str); 6] = &[
    (
        METRIC_CPU_USAGE_PROCESS_PCT_PREFIX,
        METRIC_CPU_USAGE_PROCESS_PCT_SUFFIX,
    ),
    (
        METRIC_MEMORY_PROCESS_PCT_PREFIX,
        METRIC_MEMORY_PROCESS_PCT_SUFFIX,
    ),
    (
        METRIC_STORAGE_USED_DISK_PCT_PREFIX,
        METRIC_STORAGE_USED_DISK_PCT_SUFFIX,
    ),
    (
        METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_PREFIX,
        METRIC_CONNECTIVITY_INTERFACE_RECV_BYTES_SUFFIX,
    ),
    (
        METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_PREFIX,
        METRIC_CONNECTIVITY_INTERFACE_SENT_BYTES_SUFFIX,
    ),
    (METRIC_OPERATIONAL_CRASHES_PROCESS_PREFIX, ""),
];

#[derive(Clone)]
pub struct CoreMetricKeys {
    pub string_keys: HashSet<MetricStringKey>,
    pub wildcard_pattern_keys: Vec<WildcardPattern>,
}

impl CoreMetricKeys {
    pub fn get_session_core_metrics() -> Self {
        let string_keys = HashSet::from_iter(SESSION_CORE_METRICS.map(MetricStringKey::from));
        let wildcard_pattern_keys = SESSION_CORE_WILDCARD_METIRCS
            .map(|(prefix, suffix)| WildcardPattern::new(prefix, suffix))
            .to_vec();

        Self {
            string_keys,
            wildcard_pattern_keys,
        }
    }
}
