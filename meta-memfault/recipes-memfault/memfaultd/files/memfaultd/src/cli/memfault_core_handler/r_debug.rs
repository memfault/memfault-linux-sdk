//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Read, Seek, SeekFrom};

use eyre::{eyre, Result};

use crate::cli::memfault_core_handler::ElfPtrSize;
use crate::util::mem::AsBytes;

/// "Rendezvous structures used by the run-time dynamic linker to
/// communicate details of shared object loading to the debugger."
/// See glibc's elf/link.h
/// https://sourceware.org/git/?p=glibc.git;a=blob;f=elf/link.h;h=3b5954d9818e8ea9f35638c55961f861f6ae6057

// TODO: MFLT-11643 Add support for r_debug_extended

/// The r_debug C structure from elf/link.h
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct RDebug {
    pub version: u32,
    pub r_map: ElfPtrSize,
    pub r_brk: ElfPtrSize,
    pub r_state: u32,
    pub r_ldbase: ElfPtrSize,
}

/// The link_map C structure from elf/link.h
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct LinkMap {
    pub l_addr: ElfPtrSize,
    /// Pointer to C-string.
    pub l_name: ElfPtrSize,
    pub l_ld: ElfPtrSize,
    /// Pointer to next link map.
    pub l_next: ElfPtrSize,
    pub l_prev: ElfPtrSize,
}

pub struct RDebugIter<'a, P: Read + Seek> {
    proc_mem_stream: &'a mut P,
    l_next: ElfPtrSize,
}

impl<'a, P: Read + Seek> RDebugIter<'a, P> {
    pub fn new(proc_mem_stream: &'a mut P, r_debug_addr: ElfPtrSize) -> Result<Self> {
        proc_mem_stream.seek(SeekFrom::Start(r_debug_addr as _))?;
        let mut r_debug = RDebug::default();
        // SAFETY: From the point of view of this program,
        // RDebug only contains scalar values where any value is allowed.
        let data = unsafe { r_debug.as_mut_bytes() };
        proc_mem_stream
            .read_exact(data)
            .map_err(|e| eyre!("Failed to read r_debug: {}", e))?;
        Ok(Self {
            proc_mem_stream,
            l_next: r_debug.r_map,
        })
    }

    /// Returns the next tuple of the link map's virtual address and the link map itself,
    /// or None if the end of the linked list has been reached.
    fn read_next(&mut self) -> Result<(ElfPtrSize, LinkMap)> {
        let vaddr = self.l_next;
        self.proc_mem_stream.seek(SeekFrom::Start(vaddr as _))?;
        let mut link_map = LinkMap::default();
        // SAFETY: From the point of view of this program,
        // LinkMap only contains scalar values where any value is allowed.
        let data = unsafe { link_map.as_mut_bytes() };
        self.proc_mem_stream
            .read_exact(data)
            .map_err(|e| eyre!("Failed to read link_map: {}", e))?;
        Ok((vaddr, link_map))
    }
}

impl<'a, P: Read + Seek> Iterator for RDebugIter<'a, P> {
    /// Tuple of the link map's virtual address and the link map itself.
    type Item = (ElfPtrSize, LinkMap);

    fn next(&mut self) -> Option<Self::Item> {
        if self.l_next == 0 {
            return None;
        }

        self.read_next().ok().map(|(vaddr, link_map)| {
            self.l_next = link_map.l_next;
            (vaddr, link_map)
        })
    }
}
