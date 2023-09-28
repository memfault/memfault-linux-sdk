//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Read, Seek, SeekFrom};
use std::mem::size_of;

use crate::cli::memfault_core_handler::core_elf_note::{iterate_elf_notes, ElfNote, MappedFile};
use crate::cli::memfault_core_handler::core_metadata::{write_memfault_note, CoredumpMetadata};
use crate::cli::memfault_core_handler::core_reader::CoreReader;
use crate::cli::memfault_core_handler::core_writer::{CoreWriter, SegmentData};
use crate::cli::memfault_core_handler::elf;
use crate::cli::memfault_core_handler::memory_range::{merge_memory_ranges, MemoryRange};
use crate::cli::memfault_core_handler::ElfPtrSize;
use crate::config::CoredumpCaptureStrategy;

use crate::cli::memfault_core_handler::find_elf_headers::find_elf_headers_and_build_id_note_ranges;
use crate::cli::memfault_core_handler::find_stack::find_stack;
use elf::program_header::{ProgramHeader, PT_LOAD, PT_NOTE};
use eyre::{eyre, Result};
use log::warn;

#[derive(Debug)]
pub struct CoreTransformerOptions {
    pub max_size: usize,
    pub capture_strategy: CoredumpCaptureStrategy,
}

/// Reads segments from core elf stream and memory stream and builds a core new elf file.
pub struct CoreTransformer<R, W, P>
where
    R: CoreReader,
    W: CoreWriter,
    P: Read + Seek,
{
    core_reader: R,
    core_writer: W,
    proc_mem_stream: P,
    metadata: CoredumpMetadata,
    options: CoreTransformerOptions,
}

impl<R, W, P> CoreTransformer<R, W, P>
where
    R: CoreReader,
    W: CoreWriter,
    P: Read + Seek,
{
    /// Creates an instance of `CoreTransformer` from an input stream and output stream
    pub fn new(
        core_reader: R,
        core_writer: W,
        proc_mem_stream: P,
        options: CoreTransformerOptions,
        metadata: CoredumpMetadata,
    ) -> Result<Self> {
        Ok(Self {
            core_reader,
            core_writer,
            proc_mem_stream,
            metadata,
            options,
        })
    }

    /// Reads segments from core elf stream and memory stream and builds a new elf file
    ///
    /// Reads all PT_LOAD and PT_NOTE program headers and their associated segments from the core.
    /// The memory for each PT_LOAD segment will be fetched from `/proc/<pid>/mem` and the
    /// resulting elf file will be written to `output_stream`.
    pub fn run_transformer(&mut self) -> Result<()> {
        let program_headers = self.core_reader.read_program_headers()?;
        let all_notes = self.read_all_note_segments(&program_headers);

        let segments_to_capture = match self.options.capture_strategy {
            CoredumpCaptureStrategy::KernelSelection => {
                self.kernel_selection_segments(&program_headers)
            }
            CoredumpCaptureStrategy::Threads { max_thread_size } => {
                self.threads_segments(&program_headers, &all_notes, max_thread_size)
            }
        };

        // Always copy over all note segments, regardless of the capturing strategy:
        for (ph, data) in all_notes {
            self.core_writer.add_segment(*ph, SegmentData::Buffer(data));
        }

        for ph in segments_to_capture {
            self.core_writer.add_segment(ph, SegmentData::ProcessMemory);
        }

        self.add_memfault_note()?;
        self.check_output_size()?;
        self.core_writer.write()?;

        Ok(())
    }

    /// Add note segments from the core elf to the output elf, verbatim.
    fn read_all_note_segments<'a>(
        &mut self,
        program_headers: &'a [ProgramHeader],
    ) -> Vec<(&'a ProgramHeader, Vec<u8>)> {
        program_headers
            .iter()
            .filter_map(|ph| match ph.p_type {
                PT_NOTE => match self.core_reader.read_segment_data(ph) {
                    Ok(data) => Some((ph, data)),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
    }

    /// All load segments from the original/input core elf as provided by the kernel, verbatim.
    fn kernel_selection_segments(
        &mut self,
        program_headers: &[ProgramHeader],
    ) -> Vec<ProgramHeader> {
        program_headers
            .iter()
            .filter_map(|ph| match ph.p_type {
                PT_LOAD => Some(*ph),
                _ => None,
            })
            .collect::<Vec<_>>()
    }

    /// Synthesizes segments for the stacks of all threads and adds them to the output, as well as
    /// any mandatory segments (build id notes, r_debug, etc.).
    fn threads_segments(
        &mut self,
        program_headers: &[ProgramHeader],
        all_notes: &[(&ProgramHeader, Vec<u8>)],
        max_thread_size: usize,
    ) -> Vec<ProgramHeader> {
        let parsed_notes = all_notes
            .iter()
            .flat_map(|(_, data)| iterate_elf_notes(data))
            .collect::<Vec<_>>();

        let mut mem_ranges = Vec::new();
        for note in &parsed_notes {
            match note {
                ElfNote::ProcessStatus(s) => {
                    if let Some(stack) = find_stack(&s.pr_reg, program_headers, max_thread_size) {
                        mem_ranges.push(stack);
                    } else {
                        warn!("Failed to collect stack for thread: {}", s.pr_pid);
                    }
                }
                ElfNote::File(f) => {
                    mem_ranges.extend(f.iter().filter(|f| f.page_offset == 0).flat_map(|file| {
                        match self.elf_metadata_ranges_for_mapped_file(file) {
                            Ok(ranges) => ranges,
                            Err(e) => {
                                warn!(
                                    "Failed to collect metadata for {} @ {:#x}: {}",
                                    file.path
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_else(|| "???".into()),
                                    file.start_addr,
                                    e
                                );
                                vec![]
                            }
                        }
                    }));
                }
                _ => { /* TODO */ }
            }
        }

        // Merge overlapping memory ranges and turn them into PT_LOAD program headers. As a
        // side-effect, this will also sort the program headers by vaddr.
        let merged_ranges = merge_memory_ranges(mem_ranges);
        merged_ranges.into_iter().map(ProgramHeader::from).collect()
    }

    fn elf_metadata_ranges_for_mapped_file(
        &mut self,
        file: &MappedFile,
    ) -> Result<Vec<MemoryRange>> {
        let vaddr_base = file.start_addr as ElfPtrSize;
        self.proc_mem_stream
            .seek(SeekFrom::Start(vaddr_base as u64))?;
        find_elf_headers_and_build_id_note_ranges(vaddr_base, &mut self.proc_mem_stream)
    }

    /// Check if the output file size exceeds the max size available
    fn check_output_size(&self) -> Result<()> {
        let output_size = self.core_writer.calc_output_size();
        if output_size > self.options.max_size {
            return Err(eyre!(
                "Core file size {} exceeds max size {}",
                output_size,
                self.options.max_size
            ));
        }

        Ok(())
    }

    /// Add memfault note to the core elf
    ///
    /// Contains CBOR encoded information about the system capturing the coredump. See
    /// [`CoredumpMetadata`] for more information.
    fn add_memfault_note(&mut self) -> Result<()> {
        let note_data = write_memfault_note(&self.metadata)?;
        let program_header = ProgramHeader {
            p_type: PT_NOTE,
            p_filesz: note_data.len().try_into()?,
            ..Default::default()
        };
        self.core_writer
            .add_segment(program_header, SegmentData::Buffer(note_data));

        Ok(())
    }
}

impl From<MemoryRange> for ProgramHeader {
    fn from(range: MemoryRange) -> Self {
        ProgramHeader {
            p_type: PT_LOAD,
            p_vaddr: range.start,
            p_filesz: range.size(),
            p_memsz: range.size(),
            p_align: size_of::<ElfPtrSize>() as ElfPtrSize,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod test {
    use crate::cli::memfault_core_handler::test_utils::{FakeProcMem, MockCoreWriter};
    use crate::test_utils::setup_logger;
    use crate::{
        cli::memfault_core_handler::core_reader::CoreReaderImpl, test_utils::set_snapshot_suffix,
    };
    use insta::assert_debug_snapshot;
    use rstest::rstest;
    use std::fs::File;
    use std::path::PathBuf;

    use super::*;

    #[rstest]
    #[case("kernel_selection", CoredumpCaptureStrategy::KernelSelection)]
    #[case("threads_32k", CoredumpCaptureStrategy::Threads { max_thread_size: 32 * 1024 })]
    fn test_transform(
        #[case] test_case_name: &str,
        #[case] capture_strategy: CoredumpCaptureStrategy,
        _setup_logger: (),
    ) {
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/elf-core-runtime-ld-paths.elf");
        let input_stream = File::open(&input_path).unwrap();
        let proc_mem_stream = FakeProcMem::new_from_path(&input_path).unwrap();
        let opts = CoreTransformerOptions {
            max_size: 1024 * 1024,
            capture_strategy,
        };
        let metadata = CoredumpMetadata {
            device_id: "12345678".to_string(),
            hardware_version: "evt".to_string(),
            software_type: "main".to_string(),
            software_version: "1.0.0".to_string(),
            sdk_version: "SDK_VERSION".to_string(),
            captured_time_epoch_s: 1234,
        };

        let core_reader = CoreReaderImpl::new(input_stream).unwrap();
        let mut segments = vec![];
        let mock_core_writer = MockCoreWriter::new(&mut segments);
        let mut transformer = CoreTransformer::new(
            core_reader,
            mock_core_writer,
            proc_mem_stream,
            opts,
            metadata,
        )
        .unwrap();

        transformer.run_transformer().unwrap();

        // Omit the actual data from the notes:
        let segments = segments
            .iter()
            .map(|(ph, seg)| {
                let seg = match seg {
                    SegmentData::ProcessMemory => SegmentData::ProcessMemory,
                    SegmentData::Buffer(_) => SegmentData::Buffer(vec![]),
                };
                (ph, seg)
            })
            .collect::<Vec<_>>();

        set_snapshot_suffix!("{}", test_case_name);
        assert_debug_snapshot!(segments);
    }
}
