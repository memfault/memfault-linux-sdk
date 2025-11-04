//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Read, Seek, SeekFrom};
use std::mem::size_of;

use elf::dynamic::{Dyn, DT_DEBUG};
use elf::program_header::{ProgramHeader, PT_DYNAMIC, PT_PHDR, SIZEOF_PHDR};
use eyre::{eyre, Result};
use itertools::Itertools;
use libc::PATH_MAX;
use log::{debug, warn};
use scroll::Pread;

use crate::cli::memfault_core_handler::elf;
use crate::cli::memfault_core_handler::memory_range::MemoryRange;
use crate::cli::memfault_core_handler::procfs::read_proc_mem;
use crate::cli::memfault_core_handler::r_debug::{LinkMap, RDebug, RDebugIter};
use crate::cli::memfault_core_handler::ElfPtrSize;

use super::elf_utils::read_program_headers;

/// This function attempts to find the ranges of memory in which the dynamic linker has stored the
/// information about the loaded shared objects. This is needed for our backend processing and
/// debuggers like GDB to be able to find and load the symbol files (with debug info) for those
/// loaded shared objects. In more detail, this function does the following:
///
/// - The function takes the AT_PHDR address from the auxiliary vector as input, the number of
///   program headers via AT_PHNUM, as well as the /proc/<pid>/mem stream. The AT_PHDR address is
///   the virtual address of the program headers of the main executable.
/// - The function then reads the main executable's program headers, finds the dynamic segment, and
///   reads the DT_DEBUG entry from it.
/// - The DT_DEBUG entry contains the virtual address of the r_debug structure. This is the
///   "rendezvous structure" used by the dynamic linker to communicate details of shared object
///   loading to the debugger.
/// - The r_debug structure contains the head of a linked list of link_map structures. Each link_map
///   represents a loaded shared object. The link_map structure contains the virtual address of the
///   string buffer containing the path to the shared object. The linked list is traversed to the
///   end, collecting the memory regions of the link_map structures and the string buffers along the
///   way.
///
/// # Arguments
/// * `memory_maps` - The list of the memory regions of the process's memory mappings. This
///   is used to bound the size reads from /proc/<pid>/mem, when determining the size of path strings.
/// * `output` - Memory regions for all of the aforementioned structures are added to this vector.
pub fn find_dynamic_linker_ranges<P: Read + Seek>(
    proc_mem_stream: &mut P,
    phdr_vaddr: ElfPtrSize,
    phdr_num: ElfPtrSize,
    memory_maps: &[MemoryRange],
    output: &mut Vec<MemoryRange>,
) -> Result<()> {
    debug!(
        "Detecting dynamic linker ranges from vaddr 0x{:x}",
        phdr_vaddr
    );

    let phdr = read_main_executable_phdr(proc_mem_stream, phdr_vaddr, phdr_num)?;
    let main_reloc_addr = calc_relocation_addr(phdr_vaddr, phdr);

    // Add the main executable's program headers.
    // Note: find_elf_headers_and_build_id_note_ranges() may also find this range, but that's OK.
    output.push(MemoryRange::from_start_and_size(
        main_reloc_addr + phdr.p_vaddr,
        phdr.p_memsz,
    ));

    let main_program_headers =
        read_main_exec_program_headers(proc_mem_stream, &phdr, main_reloc_addr)?;
    let dynamic_ph = find_dynamic_program_header(&main_program_headers)?;
    // Add the dynamic segment contents:
    output.push(MemoryRange::from_start_and_size(
        main_reloc_addr + dynamic_ph.p_vaddr,
        dynamic_ph.p_memsz,
    ));

    let r_debug_addr = find_r_debug_addr(proc_mem_stream, main_reloc_addr, dynamic_ph)?;
    // Add the r_debug struct itself:
    output.push(MemoryRange::from_start_and_size(
        r_debug_addr,
        size_of::<RDebug>() as ElfPtrSize,
    ));

    let mut name_vaddrs: Vec<ElfPtrSize> = vec![];

    RDebugIter::new(proc_mem_stream, r_debug_addr)?.for_each(|(vaddr, link_map)| {
        // Add the link_map node itself:
        output.push(MemoryRange::from_start_and_size(
            vaddr,
            size_of::<LinkMap>() as ElfPtrSize,
        ));

        // Stash the vaddr to the C-string with the path name of the shared object.
        // Because the proc_mem_stream is already mutably borrowed, we can't read the string buffer
        // here, so we'll do it after.
        name_vaddrs.push(link_map.l_name);
    });

    // Add the memory region for each string buffer containing the shared object's path:
    name_vaddrs.into_iter().for_each(|name_vaddr| {
        output.push(find_c_string_region(
            proc_mem_stream,
            memory_maps,
            name_vaddr,
        ));
    });

    Ok(())
}

fn find_c_string_region<P: Read + Seek>(
    proc_mem_stream: &mut P,
    memory_maps: &[MemoryRange],
    c_string_vaddr: ElfPtrSize,
) -> MemoryRange {
    // Read up to PATH_MAX bytes from the given virtual address, or until the end of the memory of
    // the memory mapping in which the address resides, whichever is smaller:
    let read_size = memory_maps
        .iter()
        .find(|r| r.contains(c_string_vaddr))
        .map_or(PATH_MAX as ElfPtrSize, |r| r.end - c_string_vaddr)
        .min(PATH_MAX as ElfPtrSize);

    read_proc_mem(proc_mem_stream, c_string_vaddr, read_size)
        .map(|data| {
            data.iter().find_position(|b| **b == 0).map_or_else(
                || MemoryRange::from_start_and_size(c_string_vaddr, read_size),
                |(idx, _)| {
                    // +1 for the NUL terminator:
                    let string_size = (idx + 1).min(read_size as usize);
                    MemoryRange::from_start_and_size(c_string_vaddr, string_size as ElfPtrSize)
                },
            )
        })
        .unwrap_or_else(|e| {
            warn!("Failed to read C-string at 0x{:x}: {}", c_string_vaddr, e);
            MemoryRange::from_start_and_size(c_string_vaddr, read_size)
        })
}

fn find_r_debug_addr<P: Read + Seek>(
    proc_mem_stream: &mut P,
    main_reloc_addr: ElfPtrSize,
    dynamic_ph: &ProgramHeader,
) -> Result<ElfPtrSize> {
    let dyn_data = read_proc_mem(
        proc_mem_stream,
        main_reloc_addr + dynamic_ph.p_vaddr,
        dynamic_ph.p_memsz,
    )
    .map_err(|e| eyre!("Failed to read dynamic segment: {}", e))?;

    let mut dyn_iter = DynIter::new(dyn_data);

    match find_dt_debug(&mut dyn_iter) {
        Some(addr) => Ok(addr),
        None => Err(eyre!("Missing DT_DEBUG entry")),
    }
}

/// Finds the virtual address of the r_debug structure in the DT_DEBUG entry, given a dynamic segment iterator.
fn find_dt_debug(dyn_iter: &mut impl Iterator<Item = Dyn>) -> Option<ElfPtrSize> {
    dyn_iter
        .find(|dyn_entry| dyn_entry.d_tag == DT_DEBUG as ElfPtrSize)
        .map(|dyn_entry| dyn_entry.d_val)
}

/// Iterator over the entries in a dynamic segment.
struct DynIter {
    data: Vec<u8>,
    offset: usize,
}

impl DynIter {
    fn new(data: Vec<u8>) -> Self {
        Self { data, offset: 0 }
    }
}
impl Iterator for DynIter {
    type Item = Dyn;

    fn next(&mut self) -> Option<Self::Item> {
        self.data.gread::<Dyn>(&mut self.offset).ok()
    }
}

/// Finds the PT_DYNAMIC program header in the main executable's program headers.
fn find_dynamic_program_header(program_headers: &[ProgramHeader]) -> Result<&ProgramHeader> {
    match program_headers.iter().find(|ph| ph.p_type == PT_DYNAMIC) {
        Some(ph) => Ok(ph),
        None => Err(eyre!("No PT_DYNAMIC found")),
    }
}

/// Reads the main executable's program headers, given the PHDR program header, relocation address,
/// and the /proc/<pid>/mem stream.
fn read_main_exec_program_headers<P: Read + Seek>(
    proc_mem_stream: &mut P,
    phdr: &ProgramHeader,
    main_reloc_addr: ElfPtrSize,
) -> Result<Vec<ProgramHeader>> {
    proc_mem_stream.seek(SeekFrom::Start((main_reloc_addr + phdr.p_vaddr) as _))?;
    let count = phdr.p_memsz / (SIZEOF_PHDR as ElfPtrSize);
    read_program_headers(proc_mem_stream, count as usize)
}

/// Reads the program header table from the main executable and searches for the `PT_PHDR`
/// header. From the ELF spec:
///
/// "The array element, if present, specifies the location and size of the program header
/// table itself, both in the file and in the memory image of the program. This segment
/// type may not occur more than once in a file. Moreover, it may occur only if the
/// program header table is part of the memory image of the program. If it is present,
/// it must precede any loadable segment entry."
fn read_main_executable_phdr<P: Read + Seek>(
    proc_mem_stream: &mut P,
    phdr_vaddr: ElfPtrSize,
    phdr_num: ElfPtrSize,
) -> Result<ProgramHeader> {
    proc_mem_stream.seek(SeekFrom::Start(phdr_vaddr as _))?;
    read_program_headers(proc_mem_stream, phdr_num as usize)?
        .into_iter()
        .find(|ph| ph.p_type == PT_PHDR)
        .ok_or_else(|| eyre!("Main executable PT_PHDR not found"))
}

/// Calculates the relocation address given the PHDR virtual address and PHDR program header.
fn calc_relocation_addr(phdr_vaddr: ElfPtrSize, phdr: ProgramHeader) -> ElfPtrSize {
    phdr_vaddr - phdr.p_vaddr
}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use std::path::PathBuf;

    use insta::assert_debug_snapshot;
    use rstest::rstest;
    use scroll::{IOwrite, Pwrite};

    use crate::cli::memfault_core_handler::procfs::ProcMaps;
    use crate::cli::memfault_core_handler::test_utils::{FakeProcMaps, FakeProcMem};

    use super::*;

    #[test]
    fn test_phdr_not_first_header() {
        let pdyn_header = build_test_program_header(PT_DYNAMIC);
        let phdr_header = build_test_program_header(PT_PHDR);

        let mut phdr_bytes = [0; SIZEOF_PHDR * 2];
        phdr_bytes.pwrite::<ProgramHeader>(pdyn_header, 0).unwrap();
        phdr_bytes
            .pwrite::<ProgramHeader>(phdr_header, SIZEOF_PHDR)
            .unwrap();

        let mut proc_mem_stream = Cursor::new(phdr_bytes);
        let actual_phdr = read_main_executable_phdr(&mut proc_mem_stream, 0, 2).unwrap();

        assert_eq!(actual_phdr, phdr_header);
    }

    #[test]
    fn test_find_dynamic_linker_ranges() {
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures/elf-core-runtime-ld-paths.elf");
        let mut proc_mem_stream = FakeProcMem::new_from_path(&input_path).unwrap();

        // The coredump fixture contains part of the main ELF executable at offset 0x2000:
        //   Type           Offset   VirtAddr           PhysAddr           FileSiz  MemSiz   Flg Align
        //   ...
        //   LOAD           0x002000 0x00005587ae8bd000 0x0000000000000000 0x001000 0x001000 R   0x1000

        // The embedded, partial ELF executable has this program table:
        //   Type           Offset   VirtAddr           PhysAddr           FileSiz  MemSiz   Flg Align
        //   PHDR           0x000040 0x0000000000000040 0x0000000000000040 0x0002d8 0x0002d8 R   0x8
        let phdr_vaddr = 0x5587ae8bd000 + 0x40;
        let phdr_num = 0x02d8 / SIZEOF_PHDR as ElfPtrSize;

        let mut fake_proc_maps = FakeProcMaps::new_from_path(&input_path).unwrap();
        let memory_maps = fake_proc_maps.get_process_maps().unwrap();
        let memory_maps_ranges: Vec<MemoryRange> =
            memory_maps.iter().map(MemoryRange::from).collect();

        let mut output = vec![];
        find_dynamic_linker_ranges(
            &mut proc_mem_stream,
            phdr_vaddr,
            phdr_num,
            &memory_maps_ranges,
            &mut output,
        )
        .unwrap();
        assert_debug_snapshot!(output);
    }

    #[rstest]
    // Happy case:
    #[case(
        b"1hello\0brave\0new\0world!!\0",
        vec![MemoryRange::from_start_and_size(0, 25)],
        1,
        MemoryRange::from_start_and_size(1, 6),  // hello\0 -> 6 bytes
    )]
    // Cannot find NUL terminator, clips to end of memory mapping region:
    #[case(
        b"1hello",
        vec![MemoryRange::from_start_and_size(0, 6)],
        1,
        MemoryRange::from_start_and_size(1, 5),
    )]
    // Clips to the end of memory mapping regions:
    #[case(
        b"1hello\0brave\0new\0world!!\0",
        vec![MemoryRange::new(0, 4)],
        1,
        MemoryRange::new(1, 4),
    )]
    // Falls back to PATH_MAX if no memory mapping region is found:
    #[case(
        b"1hello\0",
        vec![],
        1,
        MemoryRange::from_start_and_size(1, PATH_MAX as ElfPtrSize),
    )]
    // Clips to PATH_MAX if memory mapping region is longer than PATH_MAX and NUL is not found:
    #[case(
        &[b'A'; PATH_MAX as usize + 1],
        vec![MemoryRange::from_start_and_size(0, PATH_MAX as ElfPtrSize + 1)],
        0,
        MemoryRange::from_start_and_size(0, PATH_MAX as ElfPtrSize),
    )]
    fn test_find_c_string_region(
        #[case] proc_mem: &[u8],
        #[case] mmap_regions: Vec<MemoryRange>,
        #[case] c_string_vaddr: ElfPtrSize,
        #[case] expected: MemoryRange,
    ) {
        let mut proc_mem_stream = Cursor::new(proc_mem);
        assert_eq!(
            find_c_string_region(&mut proc_mem_stream, &mmap_regions, c_string_vaddr),
            expected
        );
    }

    #[rstest]
    // Empty
    #[case(vec![], vec![])]
    // Some items
    #[case(
        vec![1, 2, 3, 4],
        vec![
            Dyn { d_tag: 1, d_val: 2 },
            Dyn { d_tag: 3, d_val: 4 }
        ],
    )]
    // Partial item at the end
    #[case(
        vec![1, 2, 3],
        vec![
            Dyn { d_tag: 1, d_val: 2 },
        ],
    )]
    fn test_dyn_iter(#[case] input: Vec<ElfPtrSize>, #[case] expected: Vec<Dyn>) {
        let data = make_dyn_fixture(input);
        assert_eq!(DynIter::new(data).collect::<Vec<_>>(), expected);
    }

    fn make_dyn_fixture(values: Vec<ElfPtrSize>) -> Vec<u8> {
        let mut cursor = Cursor::new(vec![]);
        for value in values {
            cursor.iowrite::<ElfPtrSize>(value).unwrap();
        }
        cursor.into_inner()
    }

    fn build_test_program_header(p_type: u32) -> ProgramHeader {
        ProgramHeader {
            p_type,
            p_flags: 1,
            p_offset: 2,
            p_vaddr: 3,
            p_paddr: 4,
            p_filesz: 5,
            p_memsz: 6,
            p_align: 0,
        }
    }
}
