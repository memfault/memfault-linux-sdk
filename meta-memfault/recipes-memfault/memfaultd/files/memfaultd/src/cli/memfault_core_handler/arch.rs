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
        #[cfg(test)]
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
        #[cfg(test)]
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
    } else if #[cfg(all(target_arch = "arm", target_env = "gnu"))] {
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
        #[cfg(test)]
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;

        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                todo!()
            }
        }
    } else if #[cfg(all(target_arch = "arm", target_env = "musl"))] {
        use libc::c_ulong;
        #[derive(Debug, PartialEq, Eq)]
        #[repr(C)]
        pub struct user_regs {
            pub arm_r0: c_ulong,
            pub arm_r1: c_ulong,
            pub arm_r2: c_ulong,
            pub arm_r3: c_ulong,
            pub arm_r4: c_ulong,
            pub arm_r5: c_ulong,
            pub arm_r6: c_ulong,
            pub arm_r7: c_ulong,
            pub arm_r8: c_ulong,
            pub arm_r9: c_ulong,
            pub arm_r10: c_ulong,
            pub arm_fp: c_ulong,
            pub arm_ip: c_ulong,
            pub arm_sp: c_ulong,
            pub arm_lr: c_ulong,
            pub arm_pc: c_ulong,
            pub arm_cpsr: c_ulong,
            pub arm_orig_r0: c_ulong,
        }
        pub use user_regs as ElfGRegSet;
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.arm_sp as usize
        }
        pub use elf::header::EM_ARM as ELF_TARGET_MACHINE;
        #[cfg(test)]
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.arm_pc as usize
        }
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, _sp: usize)  {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            todo!()
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(_regs: &ElfGRegSet) -> Self {
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
        #[cfg(test)]
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;

        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(regs: &ElfGRegSet) -> Self {
                todo!()
            }
        }
    } else if #[cfg(any(target_arch = "mips", target_arch = "mips32r6"))] {
        // The `libc` crate does not define a `user_regs_struct` for MIPS, so we mirror the
        // kernel's NT_PRSTATUS register layout directly. The register set is an `elf_gregset_t`,
        // i.e. `[elf_greg_t; ELF_NGREG]`. On the o32 ABI `elf_greg_t` is a 32-bit word, giving a
        // 45 * 4 = 180 byte register set.
        // See arch/mips/include/asm/elf.h (ELF_NGREG) and arch/mips/include/uapi/asm/reg.h (the
        // MIPS32_EF_* slot indices used below).
        // The r6 ISA revision (`mips32r6`) keeps the o32 gregset layout, so it shares this arm
        // rather than falling through to the dummy arm and silently degrading to KernelSelection.
        const MIPS_ELF_NGREG: usize = 45;
        // Slot index of the stack pointer ($29) within the register set (MIPS32_EF_R29).
        const MIPS_EF_SP: usize = 35;
        // Slot index of the saved program counter (MIPS32_EF_CP0_EPC).
        const MIPS_EF_PC: usize = 40;

        #[derive(Debug, PartialEq, Eq)]
        #[repr(C)]
        pub struct ElfGRegSet {
            pub regs: [u32; MIPS_ELF_NGREG],
        }
        // `pr_reg` in NT_PRSTATUS must be byte-exact or the strict `ProcessStatusNote` size check
        // rejects every note and all threads are silently dropped. o32: 45 * 4 = 180 bytes.
        const _: () = assert!(size_of::<ElfGRegSet>() == 180);
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.regs[MIPS_EF_SP] as usize
        }
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.regs[MIPS_EF_PC] as usize
        }
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, _sp: usize) {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            todo!()
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        pub use elf::header::EM_MIPS as ELF_TARGET_MACHINE;
        #[cfg(test)]
        pub use elf::header::ELFCLASS32 as ELF_TARGET_CLASS;
        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(_regs: &ElfGRegSet) -> Self {
                todo!()
            }
        }
    } else if #[cfg(all(
        any(target_arch = "mips64", target_arch = "mips64r6"),
        target_pointer_width = "64"
    ))] {
        // As with 32-bit MIPS, `libc` defines no `user_regs_struct`, so we mirror the kernel's
        // NT_PRSTATUS layout. On the n64 ABI `elf_greg_t` is a 64-bit word, giving a 45 * 8 = 360
        // byte register set. Note the slot indices differ from o32: the general registers start at
        // slot 0 rather than 6. See the MIPS64_EF_* values in arch/mips/include/uapi/asm/reg.h.
        // `mips64r6` keeps the n64 gregset layout, so it shares this arm. The
        // `target_pointer_width = "64"` guard excludes the n32 ABI (32-bit pointers / ELFCLASS32),
        // which also reports `target_arch = "mips64"` but does not match this n64 layout; n32
        // builds fall through to the dummy arm and safely degrade to KernelSelection.
        const MIPS_ELF_NGREG: usize = 45;
        // Slot index of the stack pointer ($29) within the register set (MIPS64_EF_R29).
        const MIPS_EF_SP: usize = 29;
        // Slot index of the saved program counter (MIPS64_EF_CP0_EPC).
        const MIPS_EF_PC: usize = 34;

        #[derive(Debug, PartialEq, Eq)]
        #[repr(C)]
        pub struct ElfGRegSet {
            pub regs: [u64; MIPS_ELF_NGREG],
        }
        // `pr_reg` in NT_PRSTATUS must be byte-exact or the strict `ProcessStatusNote` size check
        // rejects every note and all threads are silently dropped. n64: 45 * 8 = 360 bytes.
        const _: () = assert!(size_of::<ElfGRegSet>() == 360);
        pub fn get_stack_pointer(regs: &ElfGRegSet) -> usize {
            regs.regs[MIPS_EF_SP] as usize
        }
        pub fn get_program_counter(regs: &ElfGRegSet) -> usize {
            regs.regs[MIPS_EF_PC] as usize
        }
        // The remaining register accessors are only exercised by stack unwinding /
        // stacktrace generation, which MIPS does not support (see `stacktrace_supported`,
        // which is only true for aarch64/x86_64). Thread-filtered coredumps only need the
        // stack pointer and program counter (above) to identify each thread's stack, so we
        // leave these as `todo!()` exactly as the 32-bit arm/x86 arms do.
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, _sp: usize) {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            todo!()
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        pub use elf::header::EM_MIPS as ELF_TARGET_MACHINE;
        #[cfg(test)]
        pub use elf::header::ELFCLASS64 as ELF_TARGET_CLASS;
        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(_regs: &ElfGRegSet) -> Self {
                todo!()
            }
        }
    }
    else {
        // Provide dummy symbols for unsupported architectures. This is preferable to
        // a compile time error, as we want to be able to compile memfaultd for all
        // architectures, but we don't need register access for all of them. Currently
        // these registers are only used to filter out stack memory from coredumps.
        pub const ELF_TARGET_MACHINE: u16 = 0;

        #[derive(Debug, PartialEq, Eq)]
        pub struct ElfGRegSet;
        pub fn get_stack_pointer(_regs: &ElfGRegSet) -> usize {
            0
        }
        pub fn get_program_counter(_regs: &ElfGRegSet) -> usize {
            0
        }
        pub fn set_stack_pointer(_regs: &mut HashMap<Register, usize>, _sp: usize)  {
            todo!()
        }
        pub fn get_return_register(_regs: &HashMap<Register, usize>) -> Option<usize> {
            None
        }
        pub const fn return_register_idx() -> Register {
            todo!()
        }
        impl From<&ElfGRegSet> for UnwindFrameContext {
            fn from(_regs: &ElfGRegSet) -> Self {
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
        target_arch = "mips",
        target_arch = "mips32r6",
        // n64 only: the `target_pointer_width` guard keeps n32 mips64 (which has a different
        // gregset/ELF layout) out, matching the gating on the register-set arm above.
        all(any(target_arch = "mips64", target_arch = "mips64r6"), target_pointer_width = "64"),
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
