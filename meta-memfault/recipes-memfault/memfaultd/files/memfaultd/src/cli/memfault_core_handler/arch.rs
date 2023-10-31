//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::cli::memfault_core_handler::elf;
use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(target_arch = "aarch64")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.sp as usize
        }
        pub use elf::header::EM_AARCH64 as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS64 as ELF_TARGET_CLASS;
    } else if #[cfg(target_arch = "x86_64")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.rsp as usize
        }
        pub use elf::header::EM_X86_64 as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS64 as ELF_TARGET_CLASS;
    } else if #[cfg(target_arch = "arm")] {
        pub use libc::user_regs as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.arm_sp as usize
        }
        pub use elf::header::EM_ARM as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;
    } else if #[cfg(target_arch = "x86")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.esp as usize
        }
        pub use elf::header::EM_386 as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;
    }
    else {
        // Provide dummy symbols for unsupported architectures. This is preferable to
        // a compile time error, as we want to be able to compile memfaultd for all
        // architectures, but we don't need register access for all of them. Currently
        // these registers are only used to filter out stack memory from coredumps.
        pub struct ElfGRegSet;
        pub fn get_stack_pointer(_regs: &ElfGRegSet) -> usize {
            0
        }
    }
}

// Function definitions for coredump thread filter support. If the target architecture
// is not supported, these functions will always return false.
cfg_if! {
    if #[cfg(any(
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "x86",
        target_arch = "x86_64"
    ))] {
        pub const fn coredump_thread_filter_supported() -> bool {
            true
        }
    } else {
        pub const fn coredump_thread_filter_supported() -> bool {
            false
        }
    }
}

#[cfg(target_endian = "little")]
pub use elf::header::ELFDATA2LSB as ELF_TARGET_ENDIANNESS;

#[cfg(target_endian = "big")]
pub use elf::header::ELFDATA2MSB as ELF_TARGET_ENDIANNESS;
