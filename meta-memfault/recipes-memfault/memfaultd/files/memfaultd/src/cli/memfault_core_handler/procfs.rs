//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::BTreeMap;
use std::fs::{read_dir, read_to_string, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use eyre::{eyre, Result, WrapErr};
use log::debug;
use procfs::{
    process::{MemoryMap, MemoryMaps},
    FromRead,
};

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

/// Reads the name of every thread of a process, keyed by tid.
///
/// Names are read from /proc/<pid>/task/<tid>/comm, which is where `pthread_setname_np` and
/// `prctl(PR_SET_NAME)` store them. Threads whose name cannot be read are omitted.
pub fn read_thread_names(pid: i32) -> Result<BTreeMap<u32, String>> {
    read_thread_names_from(Path::new(&format!("/proc/{}/task", pid)))
}

fn read_thread_names_from(task_dir: &Path) -> Result<BTreeMap<u32, String>> {
    let mut names = BTreeMap::new();
    for entry in read_dir(task_dir)
        .wrap_err_with(|| format!("Failed to read {}", task_dir.display()))?
        .flatten()
    {
        let Some(tid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        match read_to_string(entry.path().join("comm")) {
            Ok(comm) => {
                let name = comm.trim_end_matches('\n');
                if !name.is_empty() {
                    names.insert(tid, name.to_string());
                }
            }
            Err(e) => debug!("Failed to read name of thread {}: {}", tid, e),
        }
    }
    Ok(names)
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

        Ok(MemoryMaps::from_file(maps_file_name)
            .map_err(|e| eyre!("Failed to read /proc/{}/maps: {}", self.pid, e))?
            .0)
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

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, write};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn reads_names_keyed_by_tid() {
        let dir = tempdir().unwrap();
        for (tid, comm) in [("100", "main\n"), ("101", "worker-1\n")] {
            create_dir(dir.path().join(tid)).unwrap();
            write(dir.path().join(tid).join("comm"), comm).unwrap();
        }

        let names = read_thread_names_from(dir.path()).unwrap();

        assert_eq!(
            names,
            BTreeMap::from([(100, "main".to_string()), (101, "worker-1".to_string())])
        );
    }

    #[test]
    fn skips_unreadable_and_non_tid_entries() {
        let dir = tempdir().unwrap();
        create_dir(dir.path().join("100")).unwrap();
        create_dir(dir.path().join("not-a-tid")).unwrap();
        write(dir.path().join("not-a-tid").join("comm"), "x\n").unwrap();

        let names = read_thread_names_from(dir.path()).unwrap();

        assert!(names.is_empty());
    }

    #[test]
    fn missing_task_dir_is_an_error() {
        let dir = tempdir().unwrap();

        assert!(read_thread_names_from(&dir.path().join("missing")).is_err());
    }
}
