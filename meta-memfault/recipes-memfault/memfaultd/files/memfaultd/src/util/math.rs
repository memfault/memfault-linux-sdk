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
