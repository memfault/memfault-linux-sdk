//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::cmp::min;

use crate::cli::memfault_core_handler::arch::{get_stack_pointer, ElfGRegSet};
use crate::cli::memfault_core_handler::elf;
use crate::cli::memfault_core_handler::memory_range::MemoryRange;
use crate::cli::memfault_core_handler::ElfPtrSize;

use elf::program_header::ProgramHeader;

/// Attempts to find a MemoryRange for the stack based on the supplied register set.
/// The returned range is bound by the max_thread_size and the end of the segment in which
/// the stack is found. The assumption herein is that an anonymous memory mapping is created as
/// stack (and used exclusively as such). If the stack pointer is not contained in any segment,
/// None is returned.
pub fn find_stack(
    regs: &ElfGRegSet,
    program_headers: &[ProgramHeader],
    max_thread_size: usize,
) -> Option<MemoryRange> {
    let stack_pointer = get_stack_pointer(regs) as ElfPtrSize;

    // Iterate over all PT_LOAD segments and find the one that contains the stack
    // pointer. If the stack pointer is not contained in any PT_LOAD segment, we
    // ignore the thread.
    //
    // NOTE: This is an M*N operation, but both M(#segments) and N(#threads) are
    // likely quite small so this should be fine.
    for header in program_headers {
        let region = header.p_vaddr..header.p_vaddr.saturating_add(header.p_memsz);

        if region.contains(&stack_pointer) {
            // MFLT-11631 Handle upward stack growth in Threads memory selector
            let stack_size = min(
                region.end.saturating_sub(stack_pointer),
                max_thread_size as ElfPtrSize,
            );
            let stack_base = stack_pointer + stack_size;

            return Some(MemoryRange::new(stack_pointer, stack_base));
        }
    }

    None
}
