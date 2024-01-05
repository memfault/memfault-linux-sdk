//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Read, Seek};

use elf::program_header::{ProgramHeader, PT_NOTE};
use eyre::Result;
use log::debug;

use crate::cli::memfault_core_handler::core_elf_note::{iterate_elf_notes, ElfNote};
use crate::cli::memfault_core_handler::core_reader::{CoreReader, CoreReaderImpl};
use crate::cli::memfault_core_handler::memory_range::MemoryRange;
use crate::cli::memfault_core_handler::{elf, ElfPtrSize};

/// Detects whether the supplied stream is at an ELF file and if so, returns the ranges in the
/// stream, that contain the ELF header, program headers and build id note. The ranges are
/// relative to the stream's position when calling the function.
/// GDB requires the headers and note to be present, for debuginfod and build id based symbol file
/// lookups to work.
pub fn find_elf_headers_and_build_id_note_ranges<P: Read + Seek>(
    vaddr_base: ElfPtrSize,
    stream: &mut P,
) -> Result<Vec<MemoryRange>> {
    debug!(
        "Detecting ELF headers and build ID note ranges from vaddr 0x{:x}",
        vaddr_base
    );

    let mut elf_reader = CoreReaderImpl::new(stream)?;
    let elf_header = elf_reader.elf_header();
    let program_headers = elf_reader.read_program_headers()?;
    let build_id_note_ph = program_headers
        .iter()
        .find(|ph| ph.p_type == PT_NOTE && contains_gnu_build_id_note(&mut elf_reader, ph));

    match build_id_note_ph {
        Some(build_id_note_ph) => {
            let ranges = [
                // ELF header:
                MemoryRange::from_start_and_size(vaddr_base, elf_header.e_ehsize as ElfPtrSize),
                // Program header table:
                MemoryRange::from_start_and_size(
                    vaddr_base + elf_header.e_phoff,
                    (elf_header.e_phentsize as ElfPtrSize) * (elf_header.e_phnum as ElfPtrSize),
                ),
                // Note segment containing GNU Build ID:
                MemoryRange::from_start_and_size(
                    vaddr_base + build_id_note_ph.p_offset,
                    build_id_note_ph.p_filesz,
                ),
            ];

            // FIXME: MFLT-11635 CoreElf .py requires ELF header + build ID in single segment
            Ok(vec![MemoryRange::new(
                ranges.iter().map(|r| r.start).min().unwrap(),
                ranges.iter().map(|r| r.end).max().unwrap(),
            )])
        }
        None => Err(eyre::eyre!("Build ID note missing")),
    }
}

fn contains_gnu_build_id_note<P: Read + Seek>(
    elf_reader: &mut CoreReaderImpl<&mut P>,
    program_header: &ProgramHeader,
) -> bool {
    match elf_reader.read_segment_data(program_header) {
        Ok(data) => iterate_elf_notes(&data).any(|note| matches!(note, ElfNote::GnuBuildId(_))),
        _ => false,
    }
}

#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::{Cursor, SeekFrom};
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_err_result_if_no_elf() {
        let mut stream = Cursor::new(vec![0u8; 100]);
        assert_eq!(
            find_elf_headers_and_build_id_note_ranges(0, &mut stream)
                .unwrap_err()
                .to_string(),
            "Invalid ELF header"
        );
    }

    #[test]
    fn test_err_result_if_missing_build_id() {
        // A core.elf itself does not have a build id note:
        let mut stream = core_elf_stream();
        assert_eq!(
            find_elf_headers_and_build_id_note_ranges(0, &mut stream)
                .unwrap_err()
                .to_string(),
            "Build ID note missing"
        );
    }

    #[test]
    #[ignore = "FIXME: MFLT-11635 CoreElf .py requires ELF header + build ID in single segment"]
    fn test_ok_result_with_list_of_ranges() {
        let mut stream = exe_elf_stream();
        assert_eq!(
            find_elf_headers_and_build_id_note_ranges(0x1000, &mut stream).unwrap(),
            // Note: see offsets/sizes from the readelf -Wl output in exe_elf_stream().
            vec![
                // ELF header
                MemoryRange::from_start_and_size(0x1000, 0x40),
                // Program header table:
                MemoryRange::from_start_and_size(0x1000 + 0x40, 0x2d8),
                // Note segment with GNU Build ID
                MemoryRange::from_start_and_size(0x1000 + 0x358, 0x44),
            ]
        );
    }

    #[test]
    fn test_ok_result_with_list_of_ranges_mflt_11635_work_around() {
        let mut stream = exe_elf_stream();
        assert_eq!(
            find_elf_headers_and_build_id_note_ranges(0x1000, &mut stream).unwrap(),
            // Note: see offsets/sizes from the readelf -Wl output in exe_elf_stream().

            // FIXME: MFLT-11635 CoreElf .py requires ELF header + build ID in single segment
            vec![MemoryRange::new(0x1000, 0x1000 + 0x358 + 0x44),]
        );
    }

    fn core_elf_stream() -> File {
        File::open(core_elf_path()).unwrap()
    }

    fn exe_elf_stream() -> File {
        let mut stream = core_elf_stream();
        // The coredump fixture contains part of a captured ELF executable at offset 0x2000:
        //   Type           Offset   VirtAddr           PhysAddr           FileSiz  MemSiz   Flg Align
        //   ...
        //   LOAD           0x002000 0x00005587ae8bd000 0x0000000000000000 0x001000 0x001000 R   0x1000

        // The embedded, partial ELF executable has this program table:
        //   Type           Offset   VirtAddr           PhysAddr           FileSiz  MemSiz   Flg Align
        //   PHDR           0x000040 0x0000000000000040 0x0000000000000040 0x0002d8 0x0002d8 R   0x8
        //   INTERP         0x000318 0x0000000000000318 0x0000000000000318 0x00001c 0x00001c R   0x1
        //   LOAD           0x000000 0x0000000000000000 0x0000000000000000 0x000648 0x000648 R   0x1000
        //   LOAD           0x001000 0x0000000000001000 0x0000000000001000 0x000199 0x000199 R E 0x1000
        //   LOAD           0x002000 0x0000000000002000 0x0000000000002000 0x0000e4 0x0000e4 R   0x1000
        //   LOAD           0x002db0 0x0000000000003db0 0x0000000000003db0 0x000268 0x000270 RW  0x1000
        //   DYNAMIC        0x002dc0 0x0000000000003dc0 0x0000000000003dc0 0x000200 0x000200 RW  0x8
        //   NOTE           0x000338 0x0000000000000338 0x0000000000000338 0x000020 0x000020 R   0x8
        //   NOTE           0x000358 0x0000000000000358 0x0000000000000358 0x000044 0x000044 R   0x4
        //   GNU_PROPERTY   0x000338 0x0000000000000338 0x0000000000000338 0x000020 0x000020 R   0x8
        //   GNU_EH_FRAME   0x002008 0x0000000000002008 0x0000000000002008 0x00002c 0x00002c R   0x4
        //   GNU_STACK      0x000000 0x0000000000000000 0x0000000000000000 0x000000 0x000000 RW  0x10
        //   GNU_RELRO      0x002db0 0x0000000000003db0 0x0000000000003db0 0x000250 0x000250 R   0x1
        stream.seek(SeekFrom::Start(0x2000)).unwrap();
        stream
    }

    fn core_elf_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/elf-core-runtime-ld-paths.elf")
    }
}
