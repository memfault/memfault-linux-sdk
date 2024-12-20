//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use eyre::{eyre, Result};
use procfs::process::{MemoryMap, MemoryMaps};

use crate::cli::memfault_core_handler::memory_range::MemoryRange;
use crate::cli::memfault_core_handler::ElfPtrSize;

/// Opens /proc/<pid>/mem for reading.
pub fn proc_mem_stream(pid: i32) -> Result<File> {
    let proc_mem_stream = File::open(format!("/proc/{}/mem", pid))?;
    Ok(proc_mem_stream)
}

/// Reads memory from /proc/<pid>/mem into a buffer.
pub fn read_proc_mem<P: Read + Seek>(
    proc_mem_stream: &mut P,
    vaddr: ElfPtrSize,
    size: ElfPtrSize,
) -> Result<Vec<u8>> {
    proc_mem_stream.seek(SeekFrom::Start(vaddr as _))?;
    let mut buf = vec![0; size as usize];
    proc_mem_stream.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn read_proc_cmdline<P: Read>(cmd_line_stream: &mut P) -> Result<String> {
    let mut cmd_line_buf = Vec::new();
    cmd_line_stream.read_to_end(&mut cmd_line_buf)?;

    Ok(String::from_utf8_lossy(&cmd_line_buf).into_owned())
}

/// Wrapper trait for reading /proc/<pid>/maps.
///
/// Provides a useful abstraction that can be mocked out for testing.
pub trait ProcMaps {
    fn get_process_maps(&mut self) -> Result<Vec<MemoryMap>>;
}

/// Direct implementation of ProcMaps that reads from /proc/<pid>/maps file.
///
/// This is the default implementation used in production. It simply reads directly from
/// the file and returns the parsed memory ranges.
#[derive(Debug)]
pub struct ProcMapsImpl {
    pid: i32,
}

impl ProcMapsImpl {
    pub fn new(pid: i32) -> Self {
        Self { pid }
    }
}

impl ProcMaps for ProcMapsImpl {
    fn get_process_maps(&mut self) -> Result<Vec<MemoryMap>> {
        let maps_file_name = format!("/proc/{}/maps", self.pid);

        Ok(MemoryMaps::from_path(maps_file_name)
            .map_err(|e| eyre!("Failed to read /proc/{}/maps: {}", self.pid, e))?
            .memory_maps)
    }
}

impl From<&MemoryMap> for MemoryRange {
    fn from(m: &MemoryMap) -> Self {
        MemoryRange {
            start: m.address.0 as ElfPtrSize,
            end: m.address.1 as ElfPtrSize,
        }
    }
}
