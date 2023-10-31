//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::cmp::min;

use crate::cli::memfault_core_handler::arch::{get_stack_pointer, ElfGRegSet};
use crate::cli::memfault_core_handler::memory_range::MemoryRange;
use crate::cli::memfault_core_handler::ElfPtrSize;

use psm::StackDirection;

/// Attempts to find a MemoryRange for the stack based on the supplied register set.
/// The returned range is bound by the max_thread_size and the end of the segment in which
/// the stack is found. The assumption herein is that an anonymous memory mapping is created as
/// stack (and used exclusively as such). If the stack pointer is not contained in any segment,
/// None is returned.
pub fn find_stack(
    regs: &ElfGRegSet,
    mapped_memory_ranges: &[MemoryRange],
    max_thread_size: usize,
) -> Option<MemoryRange> {
    let stack_pointer = get_stack_pointer(regs) as ElfPtrSize;
    let stack_direction = StackDirection::new();

    find_stack_inner(
        stack_pointer,
        mapped_memory_ranges,
        max_thread_size,
        stack_direction,
    )
}

fn find_stack_inner(
    stack_pointer: ElfPtrSize,
    mapped_memory_ranges: &[MemoryRange],
    max_thread_size: usize,
    stack_direction: StackDirection,
) -> Option<MemoryRange> {
    // Iterate over all PT_LOAD segments and find the one that contains the stack
    // pointer. If the stack pointer is not contained in any PT_LOAD segment, we
    // ignore the thread.
    //
    // NOTE: This is an M*N operation, but both M(#segments) and N(#threads) are
    // likely quite small so this should be fine.
    for memory_range in mapped_memory_ranges {
        if memory_range.contains(stack_pointer as ElfPtrSize) {
            let (stack_start, stack_end) = match stack_direction {
                StackDirection::Ascending => {
                    let stack_size = min(
                        stack_pointer.saturating_sub(memory_range.start),
                        max_thread_size as ElfPtrSize,
                    );

                    let stack_base = stack_pointer - stack_size;
                    (stack_base, stack_pointer)
                }
                StackDirection::Descending => {
                    let stack_size = min(
                        memory_range.end.saturating_sub(stack_pointer),
                        max_thread_size as ElfPtrSize,
                    );

                    let stack_base = stack_pointer + stack_size;
                    (stack_pointer, stack_base)
                }
            };

            return Some(MemoryRange::new(stack_start, stack_end));
        }
    }

    None
}

#[cfg(test)]
mod test {
    use super::*;

    use rstest::rstest;

    #[rstest]
    #[case::stack_ascending(
        StackDirection::Ascending,
        0x1500,
        MemoryRange::new(0x500, 0x1500),
        0x1000
    )]
    #[case::stack_descending(
        StackDirection::Descending,
        0x1500,
        MemoryRange::new(0x1500, 0x2500),
        0x1000
    )]
    fn test_stack_calculation(
        #[case] stack_direction: StackDirection,
        #[case] stack_pointer: ElfPtrSize,
        #[case] expected_stack: MemoryRange,
        #[case] max_thread_size: usize,
    ) {
        let mapped_regions = program_header_fixture();

        let stack = find_stack_inner(
            stack_pointer,
            &mapped_regions,
            max_thread_size,
            stack_direction,
        );
        assert_eq!(stack, Some(expected_stack));
    }

    #[rstest]
    #[case::below_regions(0x0050)]
    #[case::between_regions(0x0400)]
    #[case::above_regions(0x3000)]
    fn test_stack_not_found(#[case] stack_pointer: ElfPtrSize) {
        let mapped_ranges = program_header_fixture();

        let stack = find_stack_inner(
            stack_pointer,
            &mapped_ranges,
            0x1000,
            StackDirection::Ascending,
        );
        assert!(stack.is_none());
    }

    fn program_header_fixture() -> Vec<MemoryRange> {
        vec![
            MemoryRange::from_start_and_size(0x0100, 0x0250),
            MemoryRange::from_start_and_size(0x0500, 0x2500),
        ]
    }
}
