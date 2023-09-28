//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::cmp::Ordering;
use std::io::{copy, sink, Read};

use crate::cli::memfault_core_handler::elf;

use elf::header::{Header, EI_CLASS, ELFCLASS, ELFMAG, EV_CURRENT, SELFMAG, SIZEOF_EHDR};
use elf::program_header::{ProgramHeader, SIZEOF_PHDR};
use eyre::{eyre, Result};
use log::trace;

pub trait CoreReader {
    /// Reads program headers from the input stream
    fn read_program_headers(&mut self) -> Result<Vec<ProgramHeader>>;

    /// Reads segment data from the input stream
    fn read_segment_data(&mut self, program_header: &ProgramHeader) -> Result<Vec<u8>>;
}

// Reads ELF headers and segments from a core file
pub struct CoreReaderImpl<R: Read> {
    input_stream: R,
    elf_header: Header,
    // Note: we track the cursor position manually here instead of requiring `Seek` so we can
    // support reading from stdin which doesn't implement `Seek`.
    cursor: usize,
}

impl<R: Read> CoreReader for CoreReaderImpl<R> {
    fn read_program_headers(&mut self) -> Result<Vec<ProgramHeader>> {
        let header_size = self.elf_header.e_phnum as usize * SIZEOF_PHDR;
        let mut header_buf = vec![0; header_size];
        self.skip_to_offset(self.elf_header.e_phoff as usize)?;

        let bytes_read = self.read_input(&mut header_buf)?;
        if bytes_read != header_size {
            return Err(eyre!("Failed to read program header table"));
        }

        let mut program_headers =
            ProgramHeader::from_bytes(&header_buf, self.elf_header.e_phnum as usize);

        // Sort, just in case the program headers are not sorted by offset.
        // Otherwise the read_segment_data() calls later may fail.
        program_headers.sort_by_key(|ph| ph.p_offset);

        Ok(program_headers)
    }

    fn read_segment_data(&mut self, program_header: &ProgramHeader) -> Result<Vec<u8>> {
        self.skip_to_offset(program_header.p_offset as usize)?;

        let mut buf = vec![0; program_header.p_filesz as usize];
        self.read_input(&mut buf)?;

        Ok(buf)
    }
}

impl<R: Read> CoreReaderImpl<R> {
    /// Creates an instance of `CoreReader` from an input stream
    pub fn new(mut input_stream: R) -> Result<Self> {
        let mut header_buf = [0u8; SIZEOF_EHDR];
        input_stream.read_exact(&mut header_buf)?;

        let elf_header = *Header::from_bytes(&header_buf);
        if !Self::verify_elf_header(&elf_header) {
            return Err(eyre!("Invalid ELF header"));
        }

        Ok(CoreReaderImpl {
            input_stream,
            elf_header,
            cursor: SIZEOF_EHDR,
        })
    }

    fn verify_elf_header(header: &Header) -> bool {
        &header.e_ident[0..SELFMAG] == ELFMAG
            && header.e_ident[EI_CLASS] == ELFCLASS
            && header.e_version == EV_CURRENT as u32
            && header.e_ehsize == SIZEOF_EHDR as u16
            && header.e_phentsize == SIZEOF_PHDR as u16
    }

    pub fn elf_header(&self) -> Header {
        self.elf_header
    }

    // Skip bytes in the stream if cursor is less than requested offset.
    fn skip_to_offset(&mut self, offset: usize) -> Result<()> {
        match self.cursor.cmp(&offset) {
            Ordering::Less => {
                // Skip to the segment
                let skip_size = offset - self.cursor;
                trace!("Skipping {} bytes from cursor {}", skip_size, self.cursor);
                self.skip_bytes(skip_size)?;
            }
            Ordering::Equal => {
                trace!("Already at requested offset");
            }
            Ordering::Greater => {
                return Err(eyre!("Cursor already past requested offset"));
            }
        }

        Ok(())
    }

    /// Read from input stream and update cursor position
    fn read_input(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.input_stream.read_exact(buf)?;
        self.cursor += buf.len();

        Ok(buf.len())
    }

    /// Consume specified number of bytes from the input stream
    fn skip_bytes(&mut self, bytes: usize) -> Result<()> {
        self.cursor += copy(
            &mut self.input_stream.by_ref().take(bytes as u64),
            &mut sink(),
        )? as usize;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::cli::memfault_core_handler::test_utils::build_test_header;

    use rstest::rstest;
    use scroll::Pwrite;

    #[rstest]
    #[case(0)]
    #[case(1024)] // Test with padding between header and program headers
    fn test_read_program_headers(#[case] ph_offset: usize) {
        let mut elf_header = build_test_header();
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
        input_stream
            .pwrite_with(elf_header, 0, scroll::NATIVE)
            .unwrap();
        input_stream
            .pwrite_with(load_program_header, SIZEOF_EHDR + ph_offset, scroll::NATIVE)
            .unwrap();
        input_stream
            .pwrite_with(
                note_program_header,
                SIZEOF_EHDR + SIZEOF_PHDR + ph_offset,
                scroll::NATIVE,
            )
            .unwrap();

        // Verify headers are read correctly
        let mut reader = CoreReaderImpl::new(input_stream.as_slice()).unwrap();
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

        let elf_header = build_test_header();
        let offset = offset + SIZEOF_EHDR;
        let note_program_header = ProgramHeader {
            p_type: elf::program_header::PT_NOTE,
            p_offset: offset.try_into().unwrap(),
            p_filesz: size.try_into().unwrap(),
            ..Default::default()
        };

        let mut input_stream = vec![0u8; offset + size];
        input_stream
            .pwrite_with(elf_header, 0, scroll::NATIVE)
            .unwrap();
        input_stream[offset..(offset + size)].fill(TEST_BYTE);

        let mut reader = CoreReaderImpl::new(input_stream.as_slice()).unwrap();
        let segment_data = reader.read_segment_data(&note_program_header).unwrap();

        assert_eq!(segment_data, input_stream[offset..(offset + size)]);
    }
}
