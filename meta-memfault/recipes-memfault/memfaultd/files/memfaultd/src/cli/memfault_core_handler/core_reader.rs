//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Read, Seek, SeekFrom};

use crate::cli::memfault_core_handler::arch::{ELF_TARGET_ENDIANNESS, ELF_TARGET_MACHINE};
use crate::{cli::memfault_core_handler::elf, util::io::ForwardOnlySeeker};

use elf::header::{Header, EI_CLASS, EI_DATA, ELFCLASS, ELFMAG, EV_CURRENT, SELFMAG, SIZEOF_EHDR};
use elf::program_header::{ProgramHeader, SIZEOF_PHDR};
use eyre::{eyre, Result};

pub trait CoreReader {
    /// Reads program headers from the input stream
    fn read_program_headers(&mut self) -> Result<Vec<ProgramHeader>>;

    /// Reads segment data from the input stream
    fn read_segment_data(&mut self, program_header: &ProgramHeader) -> Result<Vec<u8>>;
}

// Reads ELF headers and segments from a core file
pub struct CoreReaderImpl<R: Read> {
    input_stream: ForwardOnlySeeker<R>,
    elf_header: Header,
}

impl<R: Read> CoreReader for CoreReaderImpl<R> {
    fn read_program_headers(&mut self) -> Result<Vec<ProgramHeader>> {
        // Ignore unnecessary cast here as it is needed on 32-bit systems.
        #[allow(clippy::unnecessary_cast)]
        self.input_stream
            .seek(SeekFrom::Start(self.elf_header.e_phoff as u64))?;

        let mut program_headers =
            read_program_headers(&mut self.input_stream, self.elf_header.e_phnum as usize)?;

        // Sort, just in case the program headers are not sorted by offset.
        // Otherwise the read_segment_data() calls later may fail.
        program_headers.sort_by_key(|ph| ph.p_offset);

        Ok(program_headers)
    }

    fn read_segment_data(&mut self, program_header: &ProgramHeader) -> Result<Vec<u8>> {
        // Ignore unnecessary cast here as it is needed on 32-bit systems.
        #[allow(clippy::unnecessary_cast)]
        self.input_stream
            .seek(SeekFrom::Start(program_header.p_offset as u64))?;

        let mut buf = vec![0; program_header.p_filesz as usize];
        self.input_stream.read_exact(&mut buf)?;

        Ok(buf)
    }
}

impl<R: Read> CoreReaderImpl<R> {
    /// Creates an instance of `CoreReader` from an input stream
    pub fn new(input_stream: R) -> Result<Self> {
        let mut input_stream = ForwardOnlySeeker::new(input_stream);
        let mut header_buf = [0u8; SIZEOF_EHDR];
        input_stream.read_exact(&mut header_buf)?;

        let elf_header = *Header::from_bytes(&header_buf);
        if !Self::verify_elf_header(&elf_header) {
            return Err(eyre!("Invalid ELF header"));
        }

        Ok(CoreReaderImpl {
            input_stream,
            elf_header,
        })
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

    pub fn elf_header(&self) -> Header {
        self.elf_header
    }
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

#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::Cursor;

    use super::*;

    use crate::cli::memfault_core_handler::arch::ELF_TARGET_CLASS;
    use crate::cli::memfault_core_handler::test_utils::build_test_header;
    use elf::header::{ELFCLASSNONE, ELFDATANONE, EM_NONE};

    use rstest::rstest;
    use scroll::Pwrite;

    #[rstest]
    #[case(0)]
    #[case(1024)] // Test with padding between header and program headers
    fn test_read_program_headers(#[case] ph_offset: usize) {
        let mut elf_header =
            build_test_header(ELF_TARGET_CLASS, ELF_TARGET_ENDIANNESS, ELF_TARGET_MACHINE);
        elf_header.e_phnum = 2;
        elf_header.e_phoff = (SIZEOF_EHDR + ph_offset).try_into().unwrap();
        let load_program_header = ProgramHeader {
            p_type: elf::program_header::PT_LOAD,
            p_vaddr: 0,
            ..Default::default()
        };
        let note_program_header = ProgramHeader {
            p_type: elf::program_header::PT_NOTE,
            p_offset: 0x1000,
            ..Default::default()
        };

        // Build ELF input stream
        let mut input_stream = vec![0; SIZEOF_EHDR + 2 * SIZEOF_PHDR + ph_offset];
        input_stream.pwrite(elf_header, 0).unwrap();
        input_stream
            .pwrite(load_program_header, SIZEOF_EHDR + ph_offset)
            .unwrap();
        input_stream
            .pwrite(note_program_header, SIZEOF_EHDR + SIZEOF_PHDR + ph_offset)
            .unwrap();

        // Verify headers are read correctly
        let mut reader = CoreReaderImpl::new(Cursor::new(input_stream)).unwrap();
        let program_headers = reader.read_program_headers().unwrap();
        assert_eq!(program_headers.len(), 2);
        assert_eq!(program_headers[0], load_program_header);
        assert_eq!(program_headers[1], note_program_header);
    }

    #[rstest]
    #[case(0, 1024)]
    #[case(1024, 1024)]
    fn test_read_segment_data(#[case] offset: usize, #[case] size: usize) {
        const TEST_BYTE: u8 = 0x42;

        let elf_header =
            build_test_header(ELF_TARGET_CLASS, ELF_TARGET_ENDIANNESS, ELF_TARGET_MACHINE);
        let offset = offset + SIZEOF_EHDR;
        let note_program_header = ProgramHeader {
            p_type: elf::program_header::PT_NOTE,
            p_offset: offset.try_into().unwrap(),
            p_filesz: size.try_into().unwrap(),
            ..Default::default()
        };

        let mut input_stream = vec![0u8; offset + size];
        input_stream.pwrite(elf_header, 0).unwrap();
        input_stream[offset..(offset + size)].fill(TEST_BYTE);

        let mut reader = CoreReaderImpl::new(Cursor::new(&input_stream)).unwrap();
        let segment_data = reader.read_segment_data(&note_program_header).unwrap();

        assert_eq!(segment_data, input_stream[offset..(offset + size)]);
    }

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
        assert!(!CoreReaderImpl::<File>::verify_elf_header(&elf_header));
    }
}
