//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{copy, Read, Seek, SeekFrom, Write};

use elf::header::{
    Header, EI_CLASS, EI_DATA, EI_VERSION, ELFMAG, ET_CORE, EV_CURRENT, SELFMAG, SIZEOF_EHDR,
};
use elf::program_header::{ProgramHeader, SIZEOF_PHDR};
use eyre::Result;
use log::error;
use scroll::Pwrite;

use crate::cli::memfault_core_handler::elf;
use crate::util::math::align_up;

pub trait CoreWriter {
    // Adds a data segment to the writer.
    fn add_segment(&mut self, program_header: ProgramHeader, data: SegmentData);

    /// Writes the core elf to the output stream.
    fn write(&mut self) -> Result<()>;

    /// Calculate output coredump size.
    ///
    /// The max size is calculated as the size of the elf header, program header table, and all
    /// segment data. We take the conservative route here and calculate the size of the ELF
    /// uncompressed, even if compression is enabled. It's likely that the compressed file will
    /// be smaller, but this at least gives us a worst case estimate.
    fn calc_output_size(&self) -> usize;
}

#[derive(Debug)]
pub enum SegmentData {
    Buffer(Vec<u8>),
    ProcessMemory,
}

#[derive(Debug)]
pub struct Segment {
    program_header: ProgramHeader,
    data: SegmentData,
}

/// Creates a new ELF core file from a set of program headers and associated segment data.
pub struct CoreWriterImpl<R, W>
where
    W: Write,
    R: Read + Seek,
{
    elf_header: Header,
    data_segments: Vec<Segment>,
    process_memory: R,
    // We track the stream offset manually instead of requiring `Seek`. This allows us to use
    // stream writers, such as GZIP, which don't implement `Seek`.
    output_offset: usize,
    output_stream: W,
}

impl<R, W> CoreWriter for CoreWriterImpl<R, W>
where
    W: Write,
    R: Read + Seek,
{
    fn add_segment(&mut self, program_header: ProgramHeader, data: SegmentData) {
        self.data_segments.push(Segment {
            program_header,
            data,
        });
    }

    fn write(&mut self) -> Result<()> {
        self.write_elf_header()?;
        self.write_data_segments()?;
        Ok(())
    }

    fn calc_output_size(&self) -> usize {
        let initial = (self.elf_header.e_phnum * self.elf_header.e_phentsize
            + self.elf_header.e_ehsize) as usize;
        self.data_segments.iter().fold(initial, |acc, s| {
            align_up(
                acc + s.program_header.p_filesz as usize,
                s.program_header.p_align as usize,
            )
        })
    }
}
impl<R, W> CoreWriterImpl<R, W>
where
    W: Write,
    R: Read + Seek,
{
    /// Creates a new instance of `CoreWriter`
    pub fn new(elf_header: Header, output_stream: W, process_memory: R) -> Self {
        Self {
            elf_header,
            data_segments: Vec::new(),
            process_memory,
            output_offset: 0,
            output_stream,
        }
    }

    /// Write ELF header to output stream.
    fn write_elf_header(&mut self) -> Result<()> {
        let mut e_ident = [0u8; 16];
        e_ident[..SELFMAG].copy_from_slice(ELFMAG);
        e_ident[EI_CLASS] = self.elf_header.e_ident[EI_CLASS];
        e_ident[EI_DATA] = self.elf_header.e_ident[EI_DATA];
        e_ident[EI_VERSION] = EV_CURRENT;

        let segment_count = self.data_segments.len();
        let (pheader_size, pheader_offset) = if segment_count > 0 {
            (SIZEOF_PHDR as u16, SIZEOF_EHDR)
        } else {
            (0, 0)
        };

        let header = Header {
            e_ident,
            e_type: ET_CORE,
            e_machine: self.elf_header.e_machine,
            e_version: EV_CURRENT as u32,
            e_ehsize: SIZEOF_EHDR as u16,
            e_phentsize: pheader_size,
            e_phnum: segment_count as u16,
            e_phoff: pheader_offset.try_into()?,
            ..Default::default()
        };

        let mut bytes: Vec<u8> = vec![0; self.elf_header.e_ehsize as usize];
        self.elf_header.e_phnum = self.data_segments.len().try_into()?;
        bytes.pwrite_with(header, 0, scroll::NATIVE)?;

        Self::write_to_output(&mut self.output_stream, &bytes, &mut self.output_offset)?;
        Ok(())
    }

    /// Write program header table and associated segment data
    ///
    /// The program header table is written first, followed by the segment data. The segment data
    /// is written in the same order as the program headers, so that we don't have to seek through
    /// the output data stream.
    fn write_data_segments(&mut self) -> Result<()> {
        // Write the program header table first. For each program header, calculate the offset
        // for the associated segment data, calculate padding if necessary. We calculate the
        // offset here so that we can later write all data segments sequentially without
        // seeking.
        let mut segment_data_offset = self.elf_header.e_phoff as usize
            + self.elf_header.e_phentsize as usize * self.data_segments.len();
        for segment in &mut self.data_segments {
            let padding = calc_padding(
                segment_data_offset,
                segment.program_header.p_align.try_into()?,
            );
            segment.program_header.p_offset = (segment_data_offset + padding).try_into()?;
            let mut bytes: Vec<u8> = vec![0; self.elf_header.e_phentsize as usize];
            segment_data_offset += segment.program_header.p_filesz as usize + padding;

            bytes.pwrite_with(segment.program_header, 0, scroll::NATIVE)?;
            Self::write_to_output(&mut self.output_stream, &bytes, &mut self.output_offset)?;
        }

        // Iterate through all segments and write the data to the output stream. Zeroed padding
        // is written if the file offset is less than expected segment data offset.
        for segment in &self.data_segments {
            let cur_position = self.output_offset;
            let padding = segment.program_header.p_offset as usize - cur_position;
            Self::write_padding(&mut self.output_stream, padding, &mut self.output_offset)?;

            match &segment.data {
                SegmentData::Buffer(data) => {
                    Self::write_to_output(&mut self.output_stream, data, &mut self.output_offset)?;
                }
                SegmentData::ProcessMemory => {
                    let header = &segment.program_header;

                    match Self::read_process_memory(
                        header.p_vaddr as usize,
                        header.p_filesz as usize,
                        &mut self.process_memory,
                        &mut self.output_stream,
                    ) {
                        Ok(bytes_read) => self.output_offset += bytes_read as usize,
                        Err(e) => error!("Failed to read process memory: {}", e),
                    }
                }
            }
        }

        Ok(())
    }

    fn read_process_memory(
        addr: usize,
        len: usize,
        process_memory: &mut R,
        output_stream: &mut W,
    ) -> Result<u64> {
        process_memory.seek(SeekFrom::Start(addr as u64))?;
        let mut mem_reader = process_memory.take(len as u64);
        let bytes_read = copy(&mut mem_reader, output_stream)?;

        Ok(bytes_read)
    }

    /// Write padding if necessary
    fn write_padding(
        output_stream: &mut W,
        padding: usize,
        output_offset: &mut usize,
    ) -> Result<()> {
        if padding > 0 {
            let padding_bytes = vec![0; padding];
            Self::write_to_output(output_stream, &padding_bytes, output_offset)?;
        }
        Ok(())
    }

    /// Write to output stream and increment cursor
    fn write_to_output(
        output_stream: &mut W,
        bytes: &[u8],
        output_offset: &mut usize,
    ) -> Result<()> {
        output_stream.write_all(bytes)?;
        *output_offset += bytes.len();
        Ok(())
    }
}

fn calc_padding(offset: usize, alignment: usize) -> usize {
    if alignment <= 1 {
        return 0;
    }

    let next_addr = align_up(offset, alignment);
    next_addr - offset
}

#[cfg(test)]
mod test {
    use crate::cli::memfault_core_handler::test_utils::build_test_header;

    use super::*;

    use rstest::rstest;
    use std::io::Cursor;

    const PROC_MEM_READ_CHUNK_SIZE: usize = 1024;

    #[rstest]
    #[case(SegmentData::Buffer(vec![0xa5; 1024]), vec![])]
    #[case(SegmentData::ProcessMemory, vec![0xaa; PROC_MEM_READ_CHUNK_SIZE])]
    #[case(SegmentData::ProcessMemory, vec![0xaa; PROC_MEM_READ_CHUNK_SIZE + PROC_MEM_READ_CHUNK_SIZE / 4])]
    fn test_added_segments(#[case] segment_data: SegmentData, #[case] mem_buffer: Vec<u8>) {
        let mem_stream = Cursor::new(mem_buffer.clone());
        let mut core_writer = build_test_writer(mem_stream);

        let segment_buffer = match &segment_data {
            SegmentData::Buffer(data) => data.clone(),
            SegmentData::ProcessMemory => mem_buffer,
        };

        core_writer.add_segment(
            ProgramHeader {
                p_type: elf::program_header::PT_LOAD,
                p_offset: 0,
                p_filesz: segment_buffer.len().try_into().unwrap(),
                p_align: 0,
                ..Default::default()
            },
            segment_data,
        );

        core_writer.write().expect("Failed to write core");
        let output = core_writer.output_stream;

        let elf_header_buf = output[..SIZEOF_EHDR].try_into().unwrap();
        let elf_header = Header::from_bytes(elf_header_buf);
        assert_eq!(elf_header.e_phnum, 1);

        // Build program header table and verify correct number of headers
        let ph_table_sz = elf_header.e_phnum as usize * elf_header.e_phentsize as usize;
        let ph_header_buf = &output[SIZEOF_EHDR..(SIZEOF_EHDR + ph_table_sz)];
        let ph_headers = ProgramHeader::from_bytes(ph_header_buf, elf_header.e_phnum as usize);
        assert_eq!(ph_headers.len(), 1);

        // Verify correct program header for added segment
        assert_eq!(ph_headers[0].p_type, elf::program_header::PT_LOAD);
        assert_eq!(ph_headers[0].p_filesz as usize, segment_buffer.len());

        // Verify segment data starts after elf header and program header table
        let segment_data_offset = ph_headers[0].p_offset as usize;
        assert_eq!(segment_data_offset, SIZEOF_EHDR + ph_table_sz);

        // Verify correct segment data
        let serialized_segment_data = &output[ph_headers[0].p_offset as usize
            ..(ph_headers[0].p_offset + ph_headers[0].p_filesz) as usize];
        assert_eq!(&segment_buffer, serialized_segment_data);
    }

    #[rstest]
    #[case(vec![1024, 1024], 1024, 3072)]
    #[case(vec![2048, 1024], 512, 3584)]
    #[case(vec![2048, 1024], 0, 3136)]
    #[case(vec![2048, 1024], 1, 3136)]
    fn test_output_size_calculation(
        #[case] segment_sizes: Vec<usize>,
        #[case] alignment: usize,
        #[case] expected_size: usize,
    ) {
        let mem_stream = Vec::new();
        let mut core_writer = build_test_writer(Cursor::new(mem_stream));

        segment_sizes.iter().for_each(|size| {
            core_writer.add_segment(
                ProgramHeader {
                    p_type: elf::program_header::PT_LOAD,
                    p_filesz: *size as u64,
                    p_align: alignment.try_into().unwrap(),
                    ..Default::default()
                },
                SegmentData::ProcessMemory,
            );
        });

        let output_size = core_writer.calc_output_size();
        assert_eq!(output_size, expected_size);
    }

    fn build_test_writer(mem_stream: Cursor<Vec<u8>>) -> CoreWriterImpl<Cursor<Vec<u8>>, Vec<u8>> {
        let elf_header = build_test_header();
        let output_stream = Vec::new();
        CoreWriterImpl::new(elf_header, output_stream, mem_stream)
    }
}
