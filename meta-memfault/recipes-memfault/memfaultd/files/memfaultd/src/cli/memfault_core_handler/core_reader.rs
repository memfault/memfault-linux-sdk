//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Read, Seek, SeekFrom};

use crate::{cli::memfault_core_handler::elf, util::io::ForwardOnlySeeker};

use elf::header::Header;
use elf::program_header::{ProgramHeader, PT_NOTE};
use eyre::Result;

use super::elf_utils::{read_elf_header, read_program_headers};

pub trait CoreReader {
    /// Reads program headers from the input stream
    fn read_program_headers(&mut self) -> Result<Vec<ProgramHeader>>;

    /// Reads segment data from the input stream
    fn read_segment_data(&mut self, program_header: &ProgramHeader) -> Result<Vec<u8>>;

    /// Reads all note segments from the input stream
    fn read_all_note_segments<'a>(
        &mut self,
        program_headers: &'a [ProgramHeader],
    ) -> Vec<(&'a ProgramHeader, Vec<u8>)>;
}

// Reads ELF headers and segments from a core file
pub struct CoreReaderImpl<R: Read> {
    input_stream: ForwardOnlySeeker<R>,
    elf_header: Header,
}

impl<R: Read> CoreReader for CoreReaderImpl<R> {
    fn read_program_headers(&mut self) -> Result<Vec<ProgramHeader>> {
        self.input_stream
            .seek(SeekFrom::Start(self.elf_header.e_phoff as _))?;

        let mut program_headers =
            read_program_headers(&mut self.input_stream, self.elf_header.e_phnum as usize)?;

        // Sort, just in case the program headers are not sorted by offset.
        // Otherwise the read_segment_data() calls later may fail.
        program_headers.sort_by_key(|ph| ph.p_offset);

        Ok(program_headers)
    }

    fn read_segment_data(&mut self, program_header: &ProgramHeader) -> Result<Vec<u8>> {
        self.input_stream
            .seek(SeekFrom::Start(program_header.p_offset as _))?;

        let mut buf = vec![0; program_header.p_filesz as usize];
        self.input_stream.read_exact(&mut buf)?;

        Ok(buf)
    }

    fn read_all_note_segments<'a>(
        &mut self,
        program_headers: &'a [ProgramHeader],
    ) -> Vec<(&'a ProgramHeader, Vec<u8>)> {
        program_headers
            .iter()
            .filter_map(|ph| match ph.p_type {
                PT_NOTE => match self.read_segment_data(ph) {
                    Ok(data) => Some((ph, data)),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
    }
}

impl<R: Read> CoreReaderImpl<R> {
    /// Creates an instance of `CoreReader` from an input stream
    pub fn new(input_stream: R) -> Result<Self> {
        let mut input_stream = ForwardOnlySeeker::new(input_stream);
        let elf_header = read_elf_header(&mut input_stream)?;

        Ok(CoreReaderImpl {
            input_stream,
            elf_header,
        })
    }

    pub fn elf_header(&self) -> Header {
        self.elf_header
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;

    use super::*;

    use crate::cli::memfault_core_handler::arch::{
        ELF_TARGET_CLASS, ELF_TARGET_ENDIANNESS, ELF_TARGET_MACHINE,
    };
    use crate::cli::memfault_core_handler::test_utils::build_test_header;

    use rstest::rstest;
    use scroll::Pwrite;

    #[rstest]
    #[case(0)]
    #[case(1024)] // Test with padding between header and program headers
    fn test_read_program_headers(#[case] ph_offset: usize) {
        use elf::{header::SIZEOF_EHDR, program_header::SIZEOF_PHDR};

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
        use elf::header::SIZEOF_EHDR;

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
}
