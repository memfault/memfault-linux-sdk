//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{collections::HashMap, mem::size_of};

use super::stack_unwinder::UnwindFrameContext;
use crate::cli::memfault_core_handler::elf;

use cfg_if::cfg_if;
use gimli::{Register, Vendor};

cfg_if! {
    if #[cfg(target_arch = "aarch64")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        use gimli::AArch64;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.sp as usize
        }
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.pc as usize
        }
        pub fn get_return_register(regs: &HashMap<Register, usize>) -> Option<usize> {
            // Register X30 is the LR on AArch64
            regs.get(&AArch64::X30).copied()
        }
        pub const fn return_register_idx() -> Register {
            AArch64::X30
        }
        pub fn set_stack_pointer(regs: &mut HashMap<Register, usize>, sp: usize)  {
            regs.insert(AArch64::SP, sp);
        }
        pub use elf::header::EM_AARCH64 as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS64 as ELF_TARGET_CLASS;
        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                let mut ctx = UnwindFrameContext::default();
                ctx.registers.insert(AArch64::X0, regs.regs[0] as usize);
                ctx.registers.insert(AArch64::X1, regs.regs[1] as usize);
                ctx.registers.insert(AArch64::X2, regs.regs[2] as usize);
                ctx.registers.insert(AArch64::X3, regs.regs[3] as usize);
                ctx.registers.insert(AArch64::X4, regs.regs[4] as usize);
                ctx.registers.insert(AArch64::X5, regs.regs[5] as usize);
                ctx.registers.insert(AArch64::X6, regs.regs[6] as usize);
                ctx.registers.insert(AArch64::X7, regs.regs[7] as usize);
                ctx.registers.insert(AArch64::X8, regs.regs[8] as usize);
                ctx.registers.insert(AArch64::X9, regs.regs[9] as usize);
                ctx.registers.insert(AArch64::X10, regs.regs[10] as usize);
                ctx.registers.insert(AArch64::X11, regs.regs[11] as usize);
                ctx.registers.insert(AArch64::X12, regs.regs[12] as usize);
                ctx.registers.insert(AArch64::X13, regs.regs[13] as usize);
                ctx.registers.insert(AArch64::X14, regs.regs[14] as usize);
                ctx.registers.insert(AArch64::X15, regs.regs[15] as usize);
                ctx.registers.insert(AArch64::X16, regs.regs[16] as usize);
                ctx.registers.insert(AArch64::X17, regs.regs[17] as usize);
                ctx.registers.insert(AArch64::X18, regs.regs[18] as usize);
                ctx.registers.insert(AArch64::X19, regs.regs[19] as usize);
                ctx.registers.insert(AArch64::X20, regs.regs[20] as usize);
                ctx.registers.insert(AArch64::X21, regs.regs[21] as usize);
                ctx.registers.insert(AArch64::X22, regs.regs[22] as usize);
                ctx.registers.insert(AArch64::X23, regs.regs[23] as usize);
                ctx.registers.insert(AArch64::X24, regs.regs[24] as usize);
                ctx.registers.insert(AArch64::X25, regs.regs[25] as usize);
                ctx.registers.insert(AArch64::X26, regs.regs[26] as usize);
                ctx.registers.insert(AArch64::X27, regs.regs[27] as usize);
                ctx.registers.insert(AArch64::X28, regs.regs[28] as usize);
                ctx.registers.insert(AArch64::X29, regs.regs[29] as usize);
                ctx.registers.insert(AArch64::X30, regs.regs[30] as usize);
                ctx.registers.insert(AArch64::SP, regs.sp as usize);
                ctx.registers.insert(AArch64::PC, regs.pc as usize);

                ctx
            }
        }
    } else if #[cfg(target_arch = "x86_64")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        use gimli::X86_64;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.rsp as usize
        }
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.rip as usize
        }
        pub fn get_return_register(regs: &HashMap<Register, usize>) -> Option<usize> {
            regs.get(&X86_64::RA).copied()
        }
        pub const fn return_register_idx() -> Register {
            X86_64::RA
        }
        pub fn set_stack_pointer(regs: &mut HashMap<Register, usize>, sp: usize)  {
            regs.insert(X86_64::RSP, sp);
        }
        pub use elf::header::EM_X86_64 as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS64 as ELF_TARGET_CLASS;

        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                let mut ctx = UnwindFrameContext::default();
                ctx.registers.insert(X86_64::RAX, regs.rax as usize);
                ctx.registers.insert(X86_64::RBX, regs.rbx as usize);
                ctx.registers.insert(X86_64::RCX, regs.rcx as usize);
                ctx.registers.insert(X86_64::RDX, regs.rdx as usize);
                ctx.registers.insert(X86_64::RSI, regs.rsi as usize);
                ctx.registers.insert(X86_64::RDI, regs.rdi as usize);
                ctx.registers.insert(X86_64::RBP, regs.rbp as usize);
                ctx.registers.insert(X86_64::RSP, regs.rsp as usize);
                ctx.registers.insert(X86_64::R8, regs.r8 as usize);
                ctx.registers.insert(X86_64::R9, regs.r9 as usize);
                ctx.registers.insert(X86_64::R10, regs.r10 as usize);
                ctx.registers.insert(X86_64::R11, regs.r11 as usize);
                ctx.registers.insert(X86_64::R12, regs.r12 as usize);
                ctx.registers.insert(X86_64::R13, regs.r13 as usize);
                ctx.registers.insert(X86_64::R14, regs.r14 as usize);
                ctx.registers.insert(X86_64::R15, regs.r15 as usize);
                ctx.registers.insert(X86_64::RFLAGS, regs.eflags as usize);
                ctx.registers.insert(X86_64::CS, regs.cs as usize);
                ctx.registers.insert(X86_64::FS, regs.fs as usize);
                ctx.registers.insert(X86_64::GS, regs.gs as usize);
                ctx.registers.insert(X86_64::SS, regs.ss as usize);
                ctx.registers.insert(X86_64::DS, regs.ds as usize);
                ctx.registers.insert(X86_64::ES, regs.es as usize);
                ctx.registers.insert(X86_64::FS_BASE, regs.fs_base as usize);
                ctx.registers.insert(X86_64::GS_BASE, regs.gs_base as usize);

                ctx
            }
        }
    } else if #[cfg(target_arch = "arm")] {
        pub use libc::user_regs as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.arm_sp as usize
        }
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.arm_pc as usize
        }
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, sp: usize)  {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            todo!()
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        pub use elf::header::EM_ARM as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;

        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                todo!()
            }
        }
    } else if #[cfg(target_arch = "x86")] {
        pub use libc::user_regs_struct as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.esp as usize
        }
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.eip as usize
        }
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, sp: usize)  {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            todo!()
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        pub use elf::header::EM_386 as ELF_TARGET_MACHINE;
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;

        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                todo!()
            }
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
        pub fn get_program_counter(_regs: &ElfGRegSet) -> usize {
            0
        }
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, sp: usize)  {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            None
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                UnwindFrameContext::default()
            }
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

cfg_if! {
    if #[cfg(target_arch = "aarch64")] {
        pub fn get_vendor() -> Vendor {
            Vendor::AArch64
        }
    } else {
        pub fn get_vendor() -> Vendor {
            Vendor::Default
        }
    }
}

// Function definitions for stacktrace support. If the target architecture is not
// supported, these functions will always return false.
cfg_if! {
    if #[cfg(any(
        target_arch = "aarch64",
        target_arch = "x86_64"
    ))] {
        pub const fn stacktrace_supported() -> bool {
            true
        }
    } else {
        pub const fn stacktrace_supported() -> bool {
            false
        }
    }
}

#[cfg(target_endian = "little")]
pub use elf::header::ELFDATA2LSB as ELF_TARGET_ENDIANNESS;

#[cfg(target_endian = "big")]
pub use elf::header::ELFDATA2MSB as ELF_TARGET_ENDIANNESS;

pub const WORD_SIZE: usize = size_of::<usize>();
