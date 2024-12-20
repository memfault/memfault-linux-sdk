//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Utilities for reading ELF files

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
};

use super::elf::section_header::SectionHeader;
use super::{
    core_elf_note::iterate_elf_notes,
    elf::header::{Header, EI_CLASS, EI_DATA, ELFCLASS, ELFMAG, EV_CURRENT, SELFMAG, SIZEOF_EHDR},
};
use super::{
    core_elf_note::ElfNote,
    elf::program_header::{ProgramHeader, SIZEOF_PHDR},
};
use crate::cli::memfault_core_handler::arch::{ELF_TARGET_ENDIANNESS, ELF_TARGET_MACHINE};

use eyre::{eyre, Result};
use libc::{PF_X, PT_NOTE};
use scroll::Pread;

/// Read the ELF header from an input stream.
///
/// Will return an error if the header is invalid, or the stream is too short.
pub fn read_elf_header<R: Read>(input_stream: &mut R) -> Result<Header> {
    let mut header_buf = [0u8; SIZEOF_EHDR];
    input_stream.read_exact(&mut header_buf)?;

    let elf_header = *Header::from_bytes(&header_buf);
    if !verify_elf_header(&elf_header) {
        return Err(eyre!("Invalid ELF header"));
    }

    Ok(elf_header)
}

/// Get the base execution address of the provided ELF file.
///
/// The same as `get_base_addr_from_reader`, but reads the program headers from the input stream.
pub fn get_base_addr_from_reader<R: Read + Seek>(
    input_stream: &mut R,
    elf_header: &Header,
) -> Result<usize> {
    input_stream.seek(SeekFrom::Start(elf_header.e_phoff as _))?;
    let program_headers = read_program_headers(input_stream, elf_header.e_phnum as usize)?;

    get_base_address_from_program_headers(&program_headers)
}

/// Reads `count` ELF program headers from the provided input stream.
pub fn read_program_headers<R: Read>(
    input_stream: &mut R,
    count: usize,
) -> Result<Vec<ProgramHeader>> {
    let size = count * SIZEOF_PHDR;
    let mut buffer = vec![0; size];
    input_stream.read_exact(&mut buffer)?;
    Ok(ProgramHeader::from_bytes(&buffer, count))
}

pub fn read_elf_notes<R: Read + Seek>(
    input_stream: &mut R,
    program_headers: &[ProgramHeader],
) -> Result<Vec<Vec<u8>>> {
    let raw_notes = program_headers
        .iter()
        .filter(|ph| ph.p_type == PT_NOTE)
        .map(|ph| {
            let mut note_data = vec![0u8; ph.p_filesz as usize];
            input_stream.seek(SeekFrom::Start(ph.p_offset as _))?;
            input_stream.read_exact(&mut note_data)?;

            Ok(note_data)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(raw_notes)
}

/// Convert raw note data into a list of parsed ELF notes.
pub fn parse_elf_notes(raw_notes: &[Vec<u8>]) -> Result<Vec<ElfNote<'_>>> {
    let parsed_notes = raw_notes
        .iter()
        .flat_map(|data| iterate_elf_notes(data))
        .collect::<Vec<_>>();

    Ok(parsed_notes)
}

pub fn get_build_id<R: Read + Seek>(input_stream: &mut R, elf_header: &Header) -> Result<Vec<u8>> {
    input_stream.seek(SeekFrom::Start(elf_header.e_phoff as _))?;
    let program_headers = read_program_headers(input_stream, elf_header.e_phnum as usize)?;

    let raw_notes = read_elf_notes(input_stream, &program_headers)?;
    let parsed_notes = parse_elf_notes(&raw_notes)?;

    let build_id = parsed_notes
        .iter()
        .find_map(|note| match note {
            ElfNote::GnuBuildId(build_id) => Some(build_id.to_vec()),
            _ => None,
        })
        .ok_or_else(|| eyre!("Failed to find GNU Build ID note"))?;

    Ok(build_id)
}

/// Finds the base execution address from the provided program headers.
///
/// This is the address of the first executable segment. This address is used to determine the
/// offset of the program when ASLR is enabled.
pub fn get_base_address_from_program_headers(program_headers: &[ProgramHeader]) -> Result<usize> {
    program_headers
        .iter()
        .find_map(|ph| {
            if ph.p_flags & PF_X != 0 {
                Some(ph.p_vaddr as usize)
            } else {
                None
            }
        })
        .ok_or_else(|| eyre!("No executable segment found"))
}

#[derive(Debug, Clone)]
/// ELF section representation.
///
/// Contains the section header and the section data.
pub struct Section {
    pub header: SectionHeader,
    pub data: Vec<u8>,
}

/// ELF section map utility.
///
/// Provides the ability to read sections from an ELF file by name. The sections are streamed
/// whenever requested, so we do not have to load the entire file into memory. Considering
/// that the ELF contains the text, data, and bss sections, this avoids basically doubling
/// the RAM footprint of any program loading its own elf.
pub struct SectionMap<R: Read + Seek> {
    elf_reader: R,
    name_map: HashMap<String, SectionHeader>,
}

impl<R> SectionMap<R>
where
    R: Read + Seek,
{
    // Create a new section map from an ELF file and its header.
    pub fn new(mut elf_reader: R, elf_header: &Header) -> Result<Self> {
        let strtab_idx = elf_header.e_shstrndx as usize;

        let section_headers = read_section_headers(&mut elf_reader, elf_header)?;
        // Read the section header string table
        let strtab = read_section(&mut elf_reader, section_headers[strtab_idx])?;

        // Create a map of section names to section headers
        let name_map = section_headers
            .into_iter()
            .map(|sh| {
                let name = section_name_from_strtab(&strtab, sh.sh_name as usize)?;
                Ok((name, sh))
            })
            .collect::<Result<HashMap<String, SectionHeader>>>()?;

        Ok(Self {
            elf_reader,
            name_map,
        })
    }

    /// Get a section by name.
    ///
    /// This allows us to only load sections into memory that we're actually interested in
    /// and avoid the rest.
    pub fn get_section(&mut self, name: &str) -> Result<Option<Section>> {
        let header = self.name_map.get(name);
        match header {
            None => Ok(None),
            Some(header) => {
                let section = read_section(&mut self.elf_reader, *header)?;

                Ok(Some(section))
            }
        }
    }
}

/// Given a section and an offset, return the section name.
fn section_name_from_strtab(strtab: &Section, offset: usize) -> Result<String> {
    // All strings are NULL terminated, so we can just search for the next NULL byte.
    let end = strtab.data[offset..]
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(strtab.data.len() - offset);
    let name = std::str::from_utf8(&strtab.data[offset..(offset + end)])?.to_string();

    Ok(name)
}

/// Read a section from an input stream.
fn read_section<R: Read + Seek>(input_stream: &mut R, header: SectionHeader) -> Result<Section> {
    let data_size = header.sh_size as usize;

    let mut data = vec![0u8; data_size];
    input_stream.seek(SeekFrom::Start(header.sh_offset as _))?;
    input_stream.read_exact(&mut data)?;

    Ok(Section { header, data })
}

/// Read the section headers from an input stream.
fn read_section_headers<R: Read + Seek>(
    input_stream: &mut R,
    header: &Header,
) -> Result<Vec<SectionHeader>> {
    let sh_offset = header.e_shoff as _;
    let sh_count = header.e_shnum as usize;
    let sh_size = header.e_shentsize as usize;
    input_stream.seek(SeekFrom::Start(sh_offset))?;

    let mut section_header_buf = vec![0u8; sh_size];
    let mut section_headers = Vec::with_capacity(sh_count);
    for _ in 0..sh_count {
        let mut offset = 0;
        input_stream.read_exact(&mut section_header_buf)?;
        let sh = section_header_buf.gread::<SectionHeader>(&mut offset)?;

        section_headers.push(sh);
    }

    Ok(section_headers)
}

fn verify_elf_header(header: &Header) -> bool {
    &header.e_ident[0..SELFMAG] == ELFMAG
        && header.e_ident[EI_CLASS] == ELFCLASS
        && header.e_ident[EI_DATA] == ELF_TARGET_ENDIANNESS
        && header.e_version == EV_CURRENT as u32
        && header.e_ehsize == SIZEOF_EHDR as u16
        && header.e_phentsize == SIZEOF_PHDR as u16
        && header.e_machine == ELF_TARGET_MACHINE
}

#[cfg(test)]
mod test {
    use std::{fs::File, path::PathBuf};

    use super::*;

    use crate::cli::memfault_core_handler::arch::ELF_TARGET_CLASS;
    use crate::cli::memfault_core_handler::elf::header::{ELFCLASSNONE, ELFDATANONE, EM_NONE};
    use crate::cli::memfault_core_handler::test_utils::build_test_header;

    use insta::assert_debug_snapshot;
    use itertools::Itertools;
    use rstest::rstest;

    #[rstest]
    // Mismatching class (32 vs 64 bit):
    #[case(ELFCLASSNONE, ELF_TARGET_ENDIANNESS, ELF_TARGET_MACHINE)]
    // Mismatching endianness:
    #[case(ELF_TARGET_CLASS, ELFDATANONE, ELF_TARGET_MACHINE)]
    // Mismatching machine:
    #[case(ELF_TARGET_CLASS, ELF_TARGET_ENDIANNESS, EM_NONE)]
    fn test_verify_elf_header_fails_for_mismatching_arch(
        #[case] class: u8,
        #[case] endianness: u8,
        #[case] machine: u16,
    ) {
        let elf_header = build_test_header(class, endianness, machine);
        assert!(!verify_elf_header(&elf_header));
    }

    #[rstest]
    fn test_section_header_read() {
        let path = bin_path();
        let mut file = File::open(path).unwrap();

        let header = read_elf_header(&mut file).unwrap();
        let section_map = read_section_headers(&mut file, &header).unwrap();

        assert_debug_snapshot!(section_map);
    }

    #[rstest]
    fn test_section_name_read() {
        let path = bin_path();
        let mut file = File::open(path).unwrap();

        let header = read_elf_header(&mut file).unwrap();
        let section_map = SectionMap::new(&mut file, &header).unwrap();
        let names = section_map
            .name_map
            .keys()
            .map(|s| s.as_str())
            .sorted()
            .collect::<Vec<&str>>();

        assert_debug_snapshot!(names);
    }

    #[rstest]
    #[case(".text", true)]
    #[case(".data", true)]
    #[case(".bss", true)]
    #[case(".garbage", false)]
    fn test_get_section(#[case] name: &str, #[case] exists: bool) {
        let path = bin_path();
        let mut file = File::open(path).unwrap();

        let header = read_elf_header(&mut file).unwrap();
        let mut section_map = SectionMap::new(&mut file, &header).unwrap();

        let section = section_map.get_section(name).unwrap();
        assert_eq!(section.is_some(), exists);
    }

    #[test]
    fn test_get_build_id() {
        let path = bin_path();
        let mut file = File::open(path).unwrap();

        let header = read_elf_header(&mut file).unwrap();
        let build_id = get_build_id(&mut file, &header).unwrap();

        assert_eq!(build_id.len(), 20);
        assert_debug_snapshot!(build_id);
    }

    fn bin_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/simple_executable/simple_exe.elf")
    }
}
