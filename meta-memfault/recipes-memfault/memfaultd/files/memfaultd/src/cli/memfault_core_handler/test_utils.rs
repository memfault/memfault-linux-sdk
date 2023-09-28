//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::cli::memfault_core_handler::core_writer::{CoreWriter, SegmentData};
use crate::cli::memfault_core_handler::elf;
use elf::{header::Header, program_header::ProgramHeader};
// NOTE: Using the "universal" (width-agnostic types in the test):
use goblin::elf::{program_header::PT_LOAD, Elf, ProgramHeader as UniversalProgramHeader};
use std::fs::read;
use std::io::{Cursor, Error, ErrorKind, Read, Seek, SeekFrom, Take};
use std::path::Path;
use take_mut::take;

pub fn build_test_header() -> Header {
    let mut e_ident = [0u8; 16];
    e_ident[..elf::header::SELFMAG].copy_from_slice(elf::header::ELFMAG);
    e_ident[elf::header::EI_CLASS] = elf::header::ELFCLASS64;
    e_ident[elf::header::EI_DATA] = elf::header::ELFDATA2LSB;
    e_ident[elf::header::EI_VERSION] = elf::header::EV_CURRENT;

    Header {
        e_phoff: elf::header::SIZEOF_EHDR.try_into().unwrap(),
        e_phentsize: elf::program_header::SIZEOF_PHDR.try_into().unwrap(),
        e_ehsize: elf::header::SIZEOF_EHDR.try_into().unwrap(),
        e_version: elf::header::EV_CURRENT.try_into().unwrap(),
        e_phnum: 0,
        e_ident,
        ..Default::default()
    }
}

pub struct MockCoreWriter<'a> {
    pub output_size: usize,
    pub segments: &'a mut Vec<(ProgramHeader, SegmentData)>,
}

impl<'a> MockCoreWriter<'a> {
    pub fn new(segments: &mut Vec<(ProgramHeader, SegmentData)>) -> MockCoreWriter {
        MockCoreWriter {
            output_size: 0,
            segments,
        }
    }
}

impl<'a> CoreWriter for MockCoreWriter<'a> {
    fn add_segment(&mut self, program_header: ProgramHeader, data: SegmentData) {
        self.segments.push((program_header, data));
    }

    fn write(&mut self) -> eyre::Result<()> {
        Ok(())
    }

    fn calc_output_size(&self) -> usize {
        self.output_size
    }
}

/// A fake `/proc/<pid>/mem` stream that uses the memory contents from PT_LOAD segments of a
/// core.elf as the data in the `/proc/<pid>/mem` file.
pub struct FakeProcMem {
    // Note: as per the ELF spec, are expected to be sorted by v_addr:
    load_segments: Vec<UniversalProgramHeader>,
    inner: Take<Cursor<Vec<u8>>>,
}

impl FakeProcMem {
    pub fn new_from_path<P: AsRef<Path>>(core_elf_path: P) -> eyre::Result<Self> {
        let data = read(core_elf_path)?;
        Self::new(data)
    }
    pub fn new(data: Vec<u8>) -> eyre::Result<Self> {
        let elf = Elf::parse(&data)?;
        let load_segments: Vec<UniversalProgramHeader> = elf
            .program_headers
            .iter()
            .filter_map(|ph| {
                if ph.p_type == PT_LOAD {
                    Some(ph.clone())
                } else {
                    None
                }
            })
            .collect();

        Ok(FakeProcMem {
            inner: FakeProcMem::make_inner(data, &load_segments[0])?,
            load_segments,
        })
    }

    /// Creates a Read stream that corresponds to the given program header.
    /// The stream is a "view" into the segment in the coredump data buffer.
    fn make_inner(
        data: Vec<u8>,
        ph: &UniversalProgramHeader,
    ) -> eyre::Result<Take<Cursor<Vec<u8>>>> {
        let mut cursor = Cursor::new(data);
        cursor.seek(SeekFrom::Start(ph.p_offset))?;
        Ok(cursor.take(ph.p_filesz))
    }

    fn file_offset_to_vaddr(&self, offset: u64) -> Option<u64> {
        self.load_segments
            .iter()
            .find(|ph| {
                let start = ph.p_offset;
                let end = start + ph.p_filesz;
                offset >= start && offset < end
            })
            .map(|ph| ph.p_vaddr + (offset - ph.p_offset))
    }
}

impl Read for FakeProcMem {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for FakeProcMem {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        if let SeekFrom::End(_) = pos {}

        // The seek offset in /proc/<pid>/mem is the virtual address of the process memory:
        let vaddr = match pos {
            SeekFrom::Start(pos) => Ok(pos),
            SeekFrom::Current(pos) => self
                .stream_position()
                .map(|p| pos.checked_add(p as i64).unwrap() as u64),
            SeekFrom::End(_) => Err(Error::new(ErrorKind::Other, "Not implemented")),
        }
        .unwrap();

        // Find the PT_LOAD segment's program header that contains the requested vaddr:
        let ph = self.load_segments.iter().find(|ph| {
            let start = ph.p_vaddr;
            let end = start + ph.p_memsz;
            vaddr >= start && vaddr < end
        });

        match ph {
            Some(ph) => {
                // When seek() is called, always create a new inner stream that contains the
                // requested seek position (vaddr), even if the seek position is within the current
                // inner:
                take(&mut self.inner, |inner| {
                    let data = inner.into_inner().into_inner();
                    FakeProcMem::make_inner(data, ph).unwrap()
                });
                // Seek within the new inner stream to the requested vaddr:
                self.inner
                    .get_mut()
                    .seek(SeekFrom::Start(ph.p_offset + vaddr - ph.p_vaddr))
            }
            None => Err(Error::new(ErrorKind::Other, "Invalid seek position")),
        }
    }

    fn stream_position(&mut self) -> std::io::Result<u64> {
        let inner_pos = self.inner.get_mut().stream_position()?;
        self.file_offset_to_vaddr(inner_pos)
            .ok_or_else(|| Error::new(ErrorKind::Other, "Invalid stream position"))
    }
}
