//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(target_arch = "aarch64")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.sp as usize
        }
    } else if #[cfg(target_arch = "x86_64")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.rsp as usize
        }
    } else if #[cfg(target_arch = "arm")] {
        pub use libc::user_regs as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.arm_sp as usize
        }
    } else if #[cfg(target_arch = "x86")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.esp as usize
        }
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
