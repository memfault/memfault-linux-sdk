//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::cli::memfault_core_handler::ElfPtrSize;
use std::cmp::max;
use std::fmt::{Debug, Formatter};

/// Convenience struct to manage memory address ranges
#[derive(PartialEq, Eq, Clone)]
pub struct MemoryRange {
    pub start: ElfPtrSize,
    pub end: ElfPtrSize,
}

impl MemoryRange {
    pub fn new(start: ElfPtrSize, end: ElfPtrSize) -> Self {
        Self { start, end }
    }

    pub fn from_start_and_size(start: ElfPtrSize, size: ElfPtrSize) -> Self {
        Self {
            start,
            end: start + size,
        }
    }

    /// Returns true if the two ranges overlap
    pub fn overlaps(&self, other: &Self) -> bool {
        self.start <= other.end && self.end > other.start
    }

    pub fn size(&self) -> ElfPtrSize {
        self.end - self.start
    }

    pub fn contains(&self, addr: ElfPtrSize) -> bool {
        self.start <= addr && addr < self.end
    }
}

impl Debug for MemoryRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryRange")
            .field("start", &format!("{:#x}", self.start))
            .field("end", &format!("{:#x}", self.end))
            .finish()
    }
}

/// Merge overlapping memory ranges.
///
/// This is used to merge memory ranges before turning them into PT_LOAD program
/// headers.
pub fn merge_memory_ranges(mut ranges: Vec<MemoryRange>) -> Vec<MemoryRange> {
    // First, sort by start address. This lets us merge overlapping ranges in a single pass
    // by knowing that we only need to check the last range in the merged list.
    ranges.sort_by_key(|r| r.start);

    // Next, iterate over the sorted ranges and merge overlapping ranges. If the current range
    // overlaps with the last range in the merged list, we extend the last range to include the
    // current range. Otherwise, we add the current range to the merged list.
    let mut merged_ranges: Vec<MemoryRange> = Vec::new();
    for range in ranges {
        if let Some(last) = merged_ranges.last_mut() {
            if last.overlaps(&range) {
                last.end = max(last.end, range.end);
                continue;
            }
        }
        merged_ranges.push(range);
    }

    merged_ranges
}

#[cfg(test)]
mod test {
    use rstest::rstest;

    use super::*;

    #[rstest]
    // Two ranges with matching boundaries
    #[case(
        vec![MemoryRange::new(0x1000, 0x2000), MemoryRange::new(0x2000, 0x3000)],
        vec![MemoryRange::new(0x1000, 0x2000), MemoryRange::new(0x2000, 0x3000)],
    )]
    // Two ranges with overlapping boundaries
    #[case(
        vec![MemoryRange::new(0x1000, 0x2000), MemoryRange::new(0x1500, 0x3000)],
        vec![MemoryRange::new(0x1000, 0x3000)],
    )]
    // Two ranges with non-overlapping boundaries
    #[case(
        vec![MemoryRange::new(0x1000, 0x2000), MemoryRange::new(0x3000, 0x4000)],
        vec![MemoryRange::new(0x1000, 0x2000), MemoryRange::new(0x3000, 0x4000)],
    )]
    // Three overlapping regions, unsorted
    #[case(
        vec![
            MemoryRange::new(0x1500, 0x3000),
            MemoryRange::new(0x1000, 0x2000),
            MemoryRange::new(0x3000, 0x5000),
        ],
        vec![MemoryRange::new(0x1000, 0x3000), MemoryRange::new(0x3000, 0x5000)]
    )]
    fn test_memory_range_merge(
        #[case] input: Vec<MemoryRange>,
        #[case] expected: Vec<MemoryRange>,
    ) {
        let merged = merge_memory_ranges(input);
        assert_eq!(merged, expected);
    }

    #[rstest]
    #[case(MemoryRange::new(0x1000, 0x2000), 0x1000, true)]
    #[case(MemoryRange::new(0x1000, 0x2000), 0x2000, false)]
    #[case(MemoryRange::new(0x1000, 0x2000), 0x0fff, false)]
    #[case(MemoryRange::new(0x1000, 0x2000), 0x2001, false)]
    fn test_memory_range_contains(
        #[case] input_range: MemoryRange,
        #[case] addr: ElfPtrSize,
        #[case] contains: bool,
    ) {
        assert_eq!(input_range.contains(addr), contains);
    }
}
