//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect memory fragmentation metrics from /proc/buddyinfo
//!
//! `/proc/buddyinfo` exposes the state of the Linux buddy allocator's free
//! lists. It is a small table with one row per (NUMA node, memory zone) and one
//! column per allocation *order*. A block of order `k` is `2^k` physically
//! contiguous pages, so the value in column `k` is the number of free blocks of
//! `2^k` pages currently on that zone's free list.
//!
//! Example contents (x86-64, single node):
//!
//! ```text
//! Node 0, zone      DMA      1      1      1      0      2      1      1      0      1      1      3
//! Node 0, zone    DMA32    203    210    146    120     82     53     37     22     13      7    121
//! Node 0, zone   Normal   1055    877    606    350    205     96     44     16      4      1      0
//! ```
//!
//! This is the primary signal for *external fragmentation*: when total free
//! memory is plentiful but scattered into many small blocks, the counters for
//! higher orders drop to zero and large contiguous allocations (DMA buffers,
//! hugepages, jumbo frames, higher-order `kmalloc`) start to fail even though
//! `free`-style "available memory" looks healthy. This bites embedded/IoT
//! devices that run for months without a reboot.
//!
//! We emit one gauge-style histogram per order, `memory/buddyinfo/order_<k>`,
//! whose value is the raw free-block count for that order summed across every
//! node and zone. Counts are the page readings straight from the file - we do
//! not convert to bytes.  
//!
//! The number of order columns is a kernel compile-time constant that varies
//! across kernels and architectures (historically `MAX_ORDER`, renamed
//! `MAX_PAGE_ORDER` on newer kernels), so the parser is data-driven and never
//! hardcodes the column count.
//!
//! See `man 5 proc_buddyinfo` for the file format.
use std::str::FromStr;

use eyre::{eyre, Result};
use std::fs::read_to_string;

use crate::metrics::{
    system_metrics::{SystemMetricFamilyCollector, MEMORY_METRIC_NAMESPACE},
    KeyedMetricReading, MetricStringKey,
};

const PROC_BUDDYINFO_PATH: &str = "/proc/buddyinfo";

/// Namespace prefix shared by every buddyinfo metric key.
const BUDDYINFO_METRIC_PREFIX: &str = "memory/buddyinfo";

pub struct BuddyInfoMetricsCollector;

impl BuddyInfoMetricsCollector {
    pub fn new() -> Self {
        Self
    }

    /// Read `/proc/buddyinfo` and map it into per-order free-block gauges.
    pub fn get_buddyinfo_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let content = read_to_string(PROC_BUDDYINFO_PATH)?;
        Self::buddyinfo_readings(&content)
    }

    /// Parse `/proc/buddyinfo` contents and build one histogram reading per
    /// order, summing free-block counts across every node and zone.
    fn buddyinfo_readings(content: &str) -> Result<Vec<KeyedMetricReading>> {
        let rows = parse_buddyinfo(content)?;
        let free_blocks_by_order = sum_free_blocks_by_order(&rows);

        free_blocks_by_order
            .iter()
            .enumerate()
            .map(|(order, count)| {
                let key = MetricStringKey::from_str(&format!(
                    "{}/order_{}",
                    BUDDYINFO_METRIC_PREFIX, order
                ))
                .map_err(|e| eyre!("Couldn't build buddyinfo metric key: {}", e))?;
                Ok(KeyedMetricReading::new_histogram(key, *count as f64))
            })
            .collect()
    }
}

impl SystemMetricFamilyCollector for BuddyInfoMetricsCollector {
    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_buddyinfo_metrics()
    }

    fn family_name(&self) -> &'static str {
        MEMORY_METRIC_NAMESPACE
    }
}

/// A single parsed `/proc/buddyinfo` row: the free-block counts for one
/// (NUMA node, memory zone) pair, indexed by allocation order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BuddyInfoRow {
    node: u32,
    zone: String,
    /// Number of free blocks at each order. Index `k` is order `k` (a block of
    /// `2^k` physically-contiguous pages). Length varies across kernels and
    /// architectures, so it is never hardcoded.
    free_blocks: Vec<u64>,
}

/// Parse the full contents of `/proc/buddyinfo` into one row per (node, zone).
///
/// Lines that don't match the expected shape are skipped. An error is returned
/// only if *no* rows could be parsed at all, so a missing or garbled file
/// simply causes the family to be skipped for that tick.
fn parse_buddyinfo(content: &str) -> Result<Vec<BuddyInfoRow>> {
    let rows: Vec<BuddyInfoRow> = content.lines().filter_map(parse_buddyinfo_line).collect();

    if rows.is_empty() {
        Err(eyre!("No parseable rows in {}", PROC_BUDDYINFO_PATH))
    } else {
        Ok(rows)
    }
}

/// Parse a single `/proc/buddyinfo` line, e.g.
/// `Node 0, zone   Normal   1  1  1  0  2  1  1  0  1  1  3`.
///
/// Whitespace is variable/aligned, so we split on whitespace rather than fixed
/// columns. Returns `None` if the line isn't a valid buddyinfo row.
fn parse_buddyinfo_line(line: &str) -> Option<BuddyInfoRow> {
    let mut tokens = line.split_whitespace();

    // "Node"
    if tokens.next()? != "Node" {
        return None;
    }
    // node id, e.g. "0," - strip the trailing comma
    let node = tokens.next()?.trim_end_matches(',').parse::<u32>().ok()?;
    // "zone"
    if tokens.next()? != "zone" {
        return None;
    }
    // zone name, e.g. "Normal"
    let zone = tokens.next()?.to_string();

    // The remaining tokens are the per-order free-block counts. Every remaining
    // token must be numeric for the row to be valid.
    let free_blocks = tokens
        .map(|t| t.parse::<u64>())
        .collect::<Result<Vec<u64>, _>>()
        .ok()?;

    if free_blocks.is_empty() {
        return None;
    }

    Some(BuddyInfoRow {
        node,
        zone,
        free_blocks,
    })
}

/// Sum free-block counts across every (node, zone) row, per order.
///
/// The result length is the widest row seen, so kernels/arches whose rows carry
/// differing column counts are handled without truncation. Shorter rows simply
/// contribute nothing to the higher orders.
fn sum_free_blocks_by_order(rows: &[BuddyInfoRow]) -> Vec<u64> {
    let max_orders = rows
        .iter()
        .map(|row| row.free_blocks.len())
        .max()
        .unwrap_or(0);

    let mut sums = vec![0u64; max_orders];
    for row in rows {
        for (order, count) in row.free_blocks.iter().enumerate() {
            sums[order] = sums[order].saturating_add(*count);
        }
    }

    sums
}

#[cfg(test)]
mod tests {
    use insta::assert_json_snapshot;
    use rstest::rstest;

    use super::*;

    /// Single-zone, single-node device (typical embedded target).
    const SINGLE_ZONE: &str =
        "Node 0, zone   Normal   1055    877    606    350    205     96     44     16      4      1      0";

    /// Multi-zone x86-64-style layout.
    const MULTI_ZONE: &str = "Node 0, zone      DMA      1      1      1      0      2      1      1      0      1      1      3
Node 0, zone    DMA32    203    210    146    120     82     53     37     22     13      7    121
Node 0, zone   Normal   1055    877    606    350    205     96     44     16      4      1      0";

    /// Heavily fragmented: many low-order blocks, zeros at every high order.
    const FRAGMENTED: &str =
        "Node 0, zone   Normal   4096   2048    512      0      0      0      0      0      0      0      0";

    /// Two nodes, each single-zone, with *differing column counts* (11 vs 8)
    const DIFFERING_COLUMNS: &str = "Node 0, zone   Normal   1055    877    606    350    205     96     44     16      4      1      0
Node 1, zone   Normal     10      5      2      1      0      0      1      2";

    #[rstest]
    #[case::single_zone(SINGLE_ZONE, 1)]
    #[case::multi_zone(MULTI_ZONE, 3)]
    #[case::fragmented(FRAGMENTED, 1)]
    #[case::differing_columns(DIFFERING_COLUMNS, 2)]
    fn test_parse_buddyinfo_row_count(#[case] content: &str, #[case] expected_rows: usize) {
        let rows = parse_buddyinfo(content).expect("valid buddyinfo parses");
        assert_eq!(rows.len(), expected_rows);
    }

    #[test]
    fn test_parse_buddyinfo_line_fields() {
        let row = parse_buddyinfo_line(SINGLE_ZONE).expect("valid line parses");
        assert_eq!(row.node, 0);
        assert_eq!(row.zone, "Normal");
        assert_eq!(
            row.free_blocks,
            vec![1055, 877, 606, 350, 205, 96, 44, 16, 4, 1, 0]
        );
    }

    #[rstest]
    // Missing the "zone" keyword.
    #[case("Node 0, Normal   1  2  3")]
    // A non-numeric count in the tail.
    #[case("Node 0, zone   Normal   1  2  three  4")]
    // A header/garbage line.
    #[case("this is not a buddyinfo line")]
    // No counts at all.
    #[case("Node 0, zone   Normal")]
    // Empty line.
    #[case("")]
    fn test_parse_buddyinfo_line_rejects_invalid(#[case] line: &str) {
        assert!(parse_buddyinfo_line(line).is_none());
    }

    #[test]
    fn test_parse_buddyinfo_all_invalid_is_err() {
        assert!(parse_buddyinfo("garbage\nmore garbage").is_err());
    }

    #[test]
    fn test_sum_free_blocks_across_zones() {
        let rows = parse_buddyinfo(MULTI_ZONE).expect("valid buddyinfo parses");
        let sums = sum_free_blocks_by_order(&rows);
        // Order 0: 1 + 203 + 1055, order 10: 3 + 121 + 0.
        assert_eq!(
            sums,
            vec![1259, 1088, 753, 470, 289, 150, 82, 38, 18, 9, 124]
        );
    }

    #[test]
    fn test_sum_free_blocks_differing_columns() {
        let rows = parse_buddyinfo(DIFFERING_COLUMNS).expect("valid buddyinfo parses");
        let sums = sum_free_blocks_by_order(&rows);
        // Result width follows the widest row (11); the shorter 8-column row
        // contributes nothing to orders 8..=10.
        assert_eq!(sums.len(), 11);
        assert_eq!(sums, vec![1065, 882, 608, 351, 205, 96, 45, 18, 4, 1, 0]);
    }

    #[rstest]
    #[case::single_zone(SINGLE_ZONE, "single_zone")]
    #[case::multi_zone(MULTI_ZONE, "multi_zone")]
    #[case::fragmented(FRAGMENTED, "fragmented")]
    #[case::differing_columns(DIFFERING_COLUMNS, "differing_columns")]
    fn test_buddyinfo_readings_snapshot(#[case] content: &str, #[case] snapshot_name: &str) {
        let readings = BuddyInfoMetricsCollector::buddyinfo_readings(content)
            .expect("valid buddyinfo produces readings");
        assert_json_snapshot!(snapshot_name, readings,
                              {"[].value.**.timestamp" => "[timestamp]"});
    }
}
