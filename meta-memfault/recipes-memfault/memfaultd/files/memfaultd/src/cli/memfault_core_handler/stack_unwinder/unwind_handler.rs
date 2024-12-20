//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! This module provides a high level interface for unwinding stack traces from a core file.

use std::{
    io::{Read, Seek, Write},
    sync::mpsc::Receiver,
};

use crate::cli::memfault_core_handler::{
    arch::get_program_counter,
    core_elf_note::{ElfNote, ProcessStatusNote},
};

use eyre::{eyre, Result};
use log::debug;
use nix::sys::signal::Signal;
use serde_json::to_writer;

use super::{
    eh_frame_finder::EhFrameFinder,
    stacktrace_format::{StacktraceFormat, ThreadDescriptor},
    UnwindFrameContext, Unwinder,
};

/// The UnwindHandler is responsible holding everything needed to unwind a stack.
///
/// Its main purpose is to group everything need to unwind a stack trace in one place,
/// and provide a simple interface to do so. Additionally this makes it easier to test
/// the unwinding logic, as all of the inputs and outputs are generic.
pub struct UnwindHandler<P, O>
where
    P: Read + Seek,
    O: Write,
{
    proc_mem_stream: P,
    output_stream: O,
}

impl<P, O> UnwindHandler<P, O>
where
    P: Read + Seek,
    O: Write,
{
    pub fn new(proc_mem_stream: P, output_stream: O) -> Self {
        Self {
            proc_mem_stream,
            output_stream,
        }
    }

    pub fn build_stacktrace<E: EhFrameFinder>(
        &mut self,
        mut eh_frame_finder: E,
        elf_notes: &[ElfNote],
        cmd_line: String,
        core_handler_logs: Receiver<String>,
        crashing_tid: i32,
        signal: i32,
    ) -> Result<()> {
        let mut unwinder = Unwinder::new(&mut eh_frame_finder);

        // Collect all prstatus notes from the core file. These will act as the starting points
        // for unwinding each stack.
        let prstatus_notes = elf_notes
            .iter()
            .filter_map(|note| match note {
                ElfNote::ProcessStatus(s) => Some(s),
                _ => None,
            })
            .collect::<Vec<_>>();
        let signal = Signal::try_from(signal).map_err(|e| eyre!(e))?.to_string();

        // Get the stacktraces for each thread and write them to the output file.
        let threads = prstatus_notes
            .iter()
            .map(|prstatus_note| {
                (
                    prstatus_note.pr_pid,
                    self.unwind_stack(prstatus_note, &mut unwinder),
                )
            })
            .collect::<Vec<_>>();

        let thread_descriptors = threads
            .into_iter()
            .map(|(tid, pcs)| {
                let active = tid as i32 == crashing_tid;
                println!("Thread {} active: {}", tid, active);
                ThreadDescriptor::new(active, pcs)
            })
            .collect::<Vec<_>>();
        let symbol_descriptors = eh_frame_finder.get_symbol_file_descriptors()?;
        let trace_logs = core_handler_logs.try_iter().collect();
        let backtrace = StacktraceFormat::new(
            signal,
            cmd_line,
            symbol_descriptors,
            thread_descriptors,
            trace_logs,
        );

        to_writer(&mut self.output_stream, &backtrace)?;

        Ok(())
    }

    fn unwind_stack<E: EhFrameFinder>(
        &mut self,
        prstatus_note: &ProcessStatusNote,
        unwinder: &mut Unwinder<E>,
    ) -> Vec<usize> {
        let regs = &prstatus_note.pr_reg;
        let pc = get_program_counter(regs);

        let mut ctx = UnwindFrameContext::from(regs);
        if let Err(e) = unwinder.unwind_stack(pc, &mut ctx, &mut self.proc_mem_stream) {
            debug!("Stack unwinding aborted: {}", e);
        }

        ctx.pc_stack
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::{fs::File, path::PathBuf, sync::mpsc::channel};

    use crate::cli::memfault_core_handler::{
        core_elf_note::iterate_elf_notes,
        core_reader::{CoreReader, CoreReaderImpl},
        elf_utils::{read_elf_header, SectionMap},
        stack_unwinder::{eh_frame_finder::EhFrameInfo, stacktrace_format::SymbolFileDescriptor},
        test_utils::FakeProcMem,
    };

    use eyre::eyre;
    use insta::assert_json_snapshot;
    use serde_json::from_reader;

    // The crashing thread id from the simple exe core file.
    const CRASHING_TID: i32 = 3518029;

    #[test]
    fn test_unwind_stack() {
        let core_path = core_path();
        let core_file = File::open(core_path.clone()).unwrap();
        let mut core_reader = CoreReaderImpl::new(core_file).unwrap();
        let proc_mem = FakeProcMem::new_from_path(core_path).unwrap();

        let program_headers = core_reader.read_program_headers().unwrap();
        let all_notes = core_reader.read_all_note_segments(&program_headers);
        let parsed_notes = all_notes
            .iter()
            .flat_map(|(_, data)| iterate_elf_notes(data))
            .collect::<Vec<_>>();

        let eh_frame_finder = SimpleEhFrameFinder::new();

        let mut output_buf: Vec<u8> = Vec::new();
        let mut unwind_handler = UnwindHandler::new(proc_mem, &mut output_buf);
        let (tx, rx) = channel();
        tx.send("Ranger".to_string()).unwrap();
        tx.send("Daisy".to_string()).unwrap();
        tx.send("Carlton".to_string()).unwrap();
        unwind_handler
            .build_stacktrace(
                eh_frame_finder,
                &parsed_notes,
                "simple_exe".to_string(),
                rx,
                CRASHING_TID,
                11,
            )
            .unwrap();

        let stacktrace: StacktraceFormat = from_reader(&output_buf[..]).unwrap();
        assert_json_snapshot!(stacktrace);
    }

    struct SimpleEHFrameInfo {
        eh_frame_info: EhFrameInfo,
        start_addr: usize,
        end_addr: usize,
        path: String,
    }

    impl SimpleEHFrameInfo {
        fn new(
            path: PathBuf,
            start_addr: usize,
            end_addr: usize,
            compiled_base_addr: usize,
        ) -> Self {
            let mut file = File::open(path).unwrap();
            let header = read_elf_header(&mut file).unwrap();
            let mut section_map = SectionMap::new(file, &header).unwrap();

            let eh_frame = section_map
                .get_section(".eh_frame")
                .expect("Failed to find .eh_frame section")
                .unwrap();
            let eh_frame_hdr = section_map
                .get_section(".eh_frame_hdr")
                .expect("Failed to find .eh_frame_hdr section")
                .unwrap();

            let eh_frame_info =
                EhFrameInfo::new(eh_frame_hdr, eh_frame, compiled_base_addr, start_addr);

            SimpleEHFrameInfo {
                eh_frame_info,
                start_addr,
                end_addr,
                path: "test_path".to_string(),
            }
        }
    }

    impl From<&SimpleEHFrameInfo> for SymbolFileDescriptor {
        fn from(info: &SimpleEHFrameInfo) -> Self {
            SymbolFileDescriptor::new(
                info.start_addr,
                info.end_addr,
                "abcd1234".to_string(),
                info.eh_frame_info.compiled_base_addr,
                info.start_addr,
                info.path.clone(),
            )
        }
    }

    /// Represents a bespoke implementation of the `EhFrameFinder` trait for testing purposes.
    ///
    /// This allows us to load exactly the `.eh_frame` and `.eh_frame_hdr` sections we want to test
    /// the unwinder. currently this is just the ranges for the main executable and libc. All
    /// addresses were manually acquired through inspection of relevant ELF files, and the PC
    /// values in the coredump.
    struct SimpleEhFrameFinder {
        frame_info: Vec<SimpleEHFrameInfo>,
    }

    impl EhFrameFinder for SimpleEhFrameFinder {
        fn find_eh_frame(&mut self, pc: usize) -> Result<EhFrameInfo> {
            let eh_frame = self
                .frame_info
                .iter()
                .find(|info| pc >= info.start_addr && pc < info.end_addr)
                .map(|info| &info.eh_frame_info)
                .ok_or_else(|| eyre!("Failed to find eh_frame for pc: {:#x}", pc))?;

            Ok(eh_frame.clone())
        }

        fn get_symbol_file_descriptors(&self) -> Result<Vec<SymbolFileDescriptor>> {
            Ok(self.frame_info.iter().map(|info| info.into()).collect())
        }
    }

    impl SimpleEhFrameFinder {
        fn new() -> Self {
            let libc_simple_frame = SimpleEHFrameInfo::new(
                libc_ehframe_path(),
                0x75959a428000,
                0x75959a5bd000,
                0x28000,
            );
            let simple_exe_simple_frame =
                SimpleEHFrameInfo::new(simple_exe_path(), 0x63010b1cf000, 0x63010b1d0000, 0x1000);
            let frame_info = vec![libc_simple_frame, simple_exe_simple_frame];

            SimpleEhFrameFinder { frame_info }
        }
    }

    fn core_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/simple_executable/simple_exe_core.elf")
    }

    fn libc_ehframe_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/simple_executable/libc_ehframe.elf")
    }

    fn simple_exe_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/simple_executable/simple_exe.elf")
    }
}
