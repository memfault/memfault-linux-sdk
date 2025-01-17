//
// Copyright (c) Memfault, Inc.
// See License.txt for details
/// Rounds the given value up to the nearest multiple of the given alignment.
///
/// For values <= 1, the value is returned unchanged.
pub fn align_up(value: usize, alignment: usize) -> usize {
    if alignment <= 1 {
        return value;
    }
    ((value) + (alignment - 1)) & !(alignment - 1)
}

/// We need to account for potential rollovers in the
/// /proc/net/dev counters, handled by this function
pub fn counter_delta_with_overflow(current: u64, previous: u64) -> u64 {
    // The only time a counter's value would be less
    // that its previous value is if it rolled over
    // due to overflow - drop these readings that overlap
    // with an overflow
    if current < previous {
        // Need to detect if the counter rolled over at u32::MAX or u64::MAX
        current
            + ((if previous > u32::MAX as u64 {
                u64::MAX
            } else {
                u32::MAX as u64
            }) - previous)
    } else {
        current - previous
    }
}
