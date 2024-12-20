//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! This module provides an interface for mapping PC values to associated unwind information.
//!
//! The `EhFrameFinder` struct is used to find the `.eh_frame` and `.eh_frame_hdr` sections for a
//! given PC value. It does this by iterating over the mapped libraries in the `FileNote` and
//! checking if the PC value falls within the address range of the library.

use std::fs::File;
use std::{
    fmt::{self, Debug, Formatter, Write},
    path::PathBuf,
};

use eyre::{eyre, Error, Result};
use itertools::Itertools;
use log::debug;
use procfs::process::{MMPermissions, MemoryMap};

use crate::cli::memfault_core_handler::{
    elf_utils::{get_base_addr_from_reader, get_build_id, read_elf_header, Section, SectionMap},
    procfs::ProcMaps,
};

use super::stacktrace_format::SymbolFileDescriptor;

/// Represents the information of a mapped library or main executable.
///
/// This struct provides everything needed to find the `.eh_frame` and `.eh_frame_hdr` sections
/// for a given PC value.
struct MappedFileInfo {
    start_addr: usize,
    end_addr: usize,
    path: PathBuf,
    binary_info: Option<BinaryInfo>,
}

impl MappedFileInfo {
    fn binary_info(&mut self) -> Result<&mut BinaryInfo> {
        if self.binary_info.is_none() {
            let file = File::open(&self.path)?;
            self.binary_info = Some(file.try_into()?);
        }
        // Safe to unwrap because we just set it, or it was not none
        Ok(self
            .binary_info
            .as_mut()
            .expect("Binary info not initialized"))
    }
}

impl Debug for MappedFileInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedLibInfo")
            .field("start_addr", &format_args!("{:#x}", self.start_addr))
            .field("end_addr", &format_args!("{:#x}", self.end_addr))
            .finish()
    }
}

impl TryFrom<&MemoryMap> for MappedFileInfo {
    type Error = Error;

    fn try_from(memory_map: &MemoryMap) -> Result<Self, Self::Error> {
        let path = match &memory_map.pathname {
            procfs::process::MMapPath::Path(path) => path,
            _ => return Err(eyre!("Memory map does not have a pathname")),
        };

        Ok(MappedFileInfo {
            start_addr: memory_map.address.0 as usize,
            end_addr: memory_map.address.1 as usize,
            path: path.clone(),
            binary_info: None,
        })
    }
}

impl TryFrom<&MappedFileInfo> for SymbolFileDescriptor {
    type Error = Error;

    fn try_from(mapped_file: &MappedFileInfo) -> Result<Self, Self::Error> {
        match mapped_file.binary_info {
            Some(ref binary_fino) => {
                let build_id = binary_fino.build_id.iter().try_fold(
                    String::with_capacity(binary_fino.build_id.len()),
                    |mut acc, b| match write!(acc, "{:02x}", b) {
                        Ok(()) => Ok(acc),
                        Err(e) => Err(eyre!("Failed to write build_id byte: {}", e)),
                    },
                )?;

                Ok(SymbolFileDescriptor::new(
                    mapped_file.start_addr,
                    mapped_file.end_addr,
                    build_id,
                    binary_fino.compiled_base_addr,
                    mapped_file.start_addr,
                    mapped_file.path.to_string_lossy().to_string(),
                ))
            }
            None => Err(eyre!("Binary info not found")),
        }
    }
}

/// All the information loaded from the actual mapped binary.
///
/// This is really just a wrapper to avoid us having to open the file multiple times
/// when not needed.
struct BinaryInfo {
    compiled_base_addr: usize,
    build_id: Vec<u8>,
    section_map: SectionMap<File>,
}

impl TryFrom<File> for BinaryInfo {
    type Error = Error;

    fn try_from(mut file: File) -> std::result::Result<Self, Self::Error> {
        let elf_header = read_elf_header(&mut file)?;
        let compiled_base_addr = get_base_addr_from_reader(&mut file, &elf_header)?;
        let build_id = get_build_id(&mut file, &elf_header)?;

        let section_map = SectionMap::new(file, &elf_header)?;

        Ok(Self {
            compiled_base_addr,
            build_id,
            section_map,
        })
    }
}

/// Contains the raw section data for the `.eh_frame_hdr` and `.eh_frame` sections.
#[derive(Debug, Clone)]
pub struct EhFrameInfo {
    pub eh_frame_hdr: Section,
    pub eh_frame: Section,
    pub compiled_base_addr: usize,
    pub runtime_base_addr: usize,
}

impl EhFrameInfo {
    pub fn new(
        eh_frame_hdr: Section,
        eh_frame: Section,
        compiled_base_addr: usize,
        runtime_base_addr: usize,
    ) -> Self {
        Self {
            eh_frame_hdr,
            eh_frame,
            compiled_base_addr,
            runtime_base_addr,
        }
    }
}

pub trait EhFrameFinder {
    /// Finds the `.eh_frame` and `.eh_frame_hdr` sections for a given PC value.
    fn find_eh_frame(&mut self, pc: usize) -> Result<EhFrameInfo>;

    /// Returns a list of symbol file descriptors for all mapped libraries.
    fn get_symbol_file_descriptors(&self) -> Result<Vec<SymbolFileDescriptor>>;
}

/// Finds the `.eh_frame` and `.eh_frame_hdr` sections for a given PC value.
///
/// Contains a list of mapped libraries and provides a method to find the `.eh_frame` and
/// `.eh_frame_hdr` sections for a given PC value. The eh_frame info will be parsed on demand,
/// and not kept in memory.
pub struct EhFrameFinderImpl {
    mapped_libs: Vec<MappedFileInfo>,
}

impl EhFrameFinder for EhFrameFinderImpl {
    fn find_eh_frame(&mut self, pc: usize) -> Result<EhFrameInfo> {
        self.find_eh_frame(pc)
    }

    fn get_symbol_file_descriptors(&self) -> Result<Vec<SymbolFileDescriptor>> {
        self.mapped_libs
            .iter()
            // Only include libraries that were part of the stacktrace
            .filter(|lib| lib.binary_info.is_some())
            .map(|mapped_lib| mapped_lib.try_into())
            .collect()
    }
}

impl EhFrameFinderImpl {
    pub fn new<P: ProcMaps>(mut proc_maps: P) -> Result<Self> {
        let mapped_libs = proc_maps
            .get_process_maps()?
            .iter()
            .filter_map(|memory_map| {
                if !memory_map.perms.contains(MMPermissions::EXECUTE) {
                    return None;
                }
                MappedFileInfo::try_from(memory_map)
                    .map_err(|e| debug!("Failed to parse memory map: {}", e))
                    .ok()
            })
            .sorted_by(|a, b| a.start_addr.cmp(&b.start_addr))
            .collect();

        Ok(Self { mapped_libs })
    }

    /// Finds the `.eh_frame` and `.eh_frame_hdr` sections for a given PC value.
    pub fn find_eh_frame(&mut self, pc: usize) -> Result<EhFrameInfo> {
        // TODO: This could be optimized by using a binary search.
        let mapped_lib = self
            .mapped_libs
            .iter_mut()
            .find(|lib| lib.start_addr <= pc && lib.end_addr > pc)
            .ok_or_else(|| eyre!("Failed to find mapped lib for pc"))?;

        let binary_info = mapped_lib.binary_info()?;
        let eh_frame_hdr_section = binary_info
            .section_map
            .get_section(".eh_frame_hdr")?
            .ok_or_else(|| eyre!("Failed to find .eh_frame_hdr section in mapped lib"))?;
        let eh_frame_section = binary_info
            .section_map
            .get_section(".eh_frame")?
            .ok_or_else(|| eyre!("Failed to find .eh_frame section in mapped lib"))?;
        let eh_frame_info = EhFrameInfo::new(
            eh_frame_hdr_section,
            eh_frame_section,
            binary_info.compiled_base_addr,
            mapped_lib.start_addr,
        );

        Ok(eh_frame_info)
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_find_eh_frame() {
        let simple_exe_path = simple_exe_path();
        let mapped_libs = vec![
            MappedFileInfo {
                start_addr: 0x400000,
                end_addr: 0x500000,
                path: "test_path".into(),
                binary_info: Some(BinaryInfo {
                    compiled_base_addr: 0x400000,
                    build_id: vec![],
                    section_map: SectionMap::new(
                        File::open(&simple_exe_path).unwrap(),
                        &read_elf_header(&mut File::open(&simple_exe_path).unwrap()).unwrap(),
                    )
                    .unwrap(),
                }),
            },
            MappedFileInfo {
                start_addr: 0x500000,
                end_addr: 0x600000,
                path: "test_path".into(),
                binary_info: Some(BinaryInfo {
                    compiled_base_addr: 0x500000,
                    build_id: vec![],
                    section_map: SectionMap::new(
                        File::open(&simple_exe_path).unwrap(),
                        &read_elf_header(&mut File::open(simple_exe_path).unwrap()).unwrap(),
                    )
                    .unwrap(),
                }),
            },
        ];
        let mut eh_frame_finder = EhFrameFinderImpl { mapped_libs };

        let eh_frame_info = eh_frame_finder.find_eh_frame(0x450000).unwrap();
        assert_eq!(eh_frame_info.compiled_base_addr, 0x400000);
        assert_eq!(eh_frame_info.runtime_base_addr, 0x400000);

        let eh_frame_info = eh_frame_finder.find_eh_frame(0x550000).unwrap();
        assert_eq!(eh_frame_info.compiled_base_addr, 0x500000);
        assert_eq!(eh_frame_info.runtime_base_addr, 0x500000);
    }

    fn simple_exe_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/simple_executable/simple_exe.elf")
    }
}
