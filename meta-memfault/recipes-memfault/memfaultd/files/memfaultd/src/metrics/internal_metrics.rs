//
// Copyright (c) Memfault, Inc.
// See License.txt for details
// MAR cleaner metrics
pub const INTERNAL_METRIC_MAR_ENTRY_COUNT: &str = "MemfaultSdkMetric_mar_entry_count";
pub const INTERNAL_METRIC_MAR_CLEANER_DURATION: &str =
    "MemfaultSdkMetric_mar_clean_duration_seconds";
pub const INTERNAL_METRIC_MAR_ENTRIES_DELETED: &str = "MemfaultSdkMetric_mar_entries_deleted";

// HRT metrics
// As it stands today, this heartbeat metric counts the number of readings in the
// *previous* HRT report, due to the order in which they are dumped (heartbeat,
// then HRT).
pub const INTERNAL_METRIC_HRT_READING_COUNT: &str = "MemfaultSdkMetric_hrt_reading_count";
