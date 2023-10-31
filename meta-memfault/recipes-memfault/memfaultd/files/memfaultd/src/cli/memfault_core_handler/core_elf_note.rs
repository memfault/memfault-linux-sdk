//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::cmp::min;
use std::ffi::{CStr, OsStr};
use std::mem::size_of;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use eyre::{eyre, Error, Result};

// The Nhdr64 struct is incorrect in goblin. We need to open a PR with them,
// but for now going to use the 32 bit version.
use crate::cli::memfault_core_handler::arch::ElfGRegSet;
use crate::cli::memfault_core_handler::auxv::Auxv;
use crate::cli::memfault_core_handler::ElfPtrSize;
use crate::util::math::align_up;
use goblin::elf::note::Nhdr32 as Nhdr;
use goblin::elf::note::{NT_FILE, NT_GNU_BUILD_ID, NT_PRSTATUS};
use log::{error, warn};
use scroll::{Pread, Pwrite};

/// Builds an ELF note from a given name, description, and note type.
pub fn build_elf_note(name: &str, description: &[u8], note_type: u32) -> Result<Vec<u8>> {
    let name_bytes = name.as_bytes();
    let mut name_size = name_bytes.len();
    // NOTE: As per the spec, the terminating NUL byte should be included in the namesz, but 0 is
    // used for an empty string:
    if name_size > 0 {
        name_size += 1;
    }
    let note_header = Nhdr {
        n_namesz: name_size.try_into()?,
        n_descsz: description.len().try_into()?,
        n_type: note_type,
    };
    let aligned_name_size = align_up(name_size, 4);

    let header_size = size_of::<Nhdr>();
    let mut note_buffer =
        vec![0u8; header_size + aligned_name_size + align_up(description.len(), 4)];
    note_buffer.pwrite(note_header, 0)?;
    note_buffer[header_size..(header_size + name_bytes.len())].copy_from_slice(name_bytes);

    let desc_offset = header_size + aligned_name_size;
    note_buffer[desc_offset..(desc_offset + description.len())].copy_from_slice(description);

    Ok(note_buffer)
}

#[derive(Debug, PartialEq, Eq)]
/// Parsed ELF note.
///
/// Contains the deserialized ELF note description for a given note type.
/// Unknown can be used in case the note type is not supported or if parsing failed.
pub enum ElfNote<'a> {
    /// Parsed CORE::NT_PRSTATUS note.
    ProcessStatus(&'a ProcessStatusNote),
    // Parsed CORE::NT_FILE note
    File(FileNote<'a>),
    // Parsed GNU::NT_GNU_BUILD_ID note.
    GnuBuildId(&'a [u8]),
    Auxv(Auxv<'a>),
    Unknown {
        name: &'a [u8],
        note_type: u32,
        description: &'a [u8],
    },
}

const NOTE_NAME_CORE: &[u8] = b"CORE";
const NOTE_NAME_GNU: &[u8] = b"GNU";

const NT_AUXV: u32 = 6;

impl<'a> ElfNote<'a> {
    fn try_parse(name: &'a [u8], note_type: u32, description: &'a [u8]) -> Result<Option<Self>> {
        match (name, note_type) {
            (NOTE_NAME_CORE, NT_PRSTATUS) => Ok(Some(Self::ProcessStatus(description.try_into()?))),
            (NOTE_NAME_CORE, NT_FILE) => Ok(Some(Self::File(description.try_into()?))),
            (NOTE_NAME_CORE, NT_AUXV) => Ok(Some(Self::Auxv(Auxv::new(description)))),
            (NOTE_NAME_GNU, NT_GNU_BUILD_ID) => Ok(Some(Self::GnuBuildId(description))),
            _ => Ok(None),
        }
    }

    fn parse(name: &'a [u8], note_type: u32, description: &'a [u8]) -> Self {
        match Self::try_parse(name, note_type, description) {
            Ok(Some(note)) => note,
            r => {
                if r.is_err() {
                    warn!(
                        "Failed to parse ELF note: name={} type={}",
                        String::from_utf8_lossy(name),
                        note_type
                    );
                }
                Self::Unknown {
                    name,
                    note_type,
                    description,
                }
            }
        }
    }
}

/// Iterator over ELF notes in a buffer.
///
/// Only the current note is deserialized at a time.
/// This prevents us from having to make multiple copies of all notes.
pub struct ElfNoteIterator<'a> {
    note_buffer: &'a [u8],
    offset: usize,
}

impl<'a> ElfNoteIterator<'a> {
    fn new(note_buffer: &'a [u8]) -> Self {
        Self {
            note_buffer,
            offset: 0,
        }
    }

    /// Try to create an ELF note from the given buffer at the given offset.
    ///
    /// If the note type is not supported, return `None`.
    fn try_next_note(offset: &mut usize, note_buffer: &'a [u8]) -> Result<ElfNote<'a>> {
        let note_header = note_buffer.gread::<Nhdr>(offset)?;
        let name_size = note_header.n_namesz as usize;
        let aligned_name_size = align_up(name_size, 4);
        let desc_size = note_header.n_descsz as usize;
        let aligned_desc_size = align_up(desc_size, 4);

        if *offset + aligned_name_size + aligned_desc_size > note_buffer.len() {
            return Err(eyre!("Note buffer shorter than expected"));
        }

        let name = match name_size {
            // NOTE: As per the spec, the terminating NUL byte is included in the namesz,
            // but 0 is used for an empty string:
            0 => &[],
            _ => &note_buffer[*offset..(*offset + name_size - 1)],
        };
        let desc_offset = *offset + aligned_name_size;
        let desc = &note_buffer[desc_offset..(desc_offset + desc_size)];

        *offset += aligned_name_size + aligned_desc_size;
        Ok(ElfNote::parse(name, note_header.n_type, desc))
    }
}

impl<'a> Iterator for ElfNoteIterator<'a> {
    type Item = ElfNote<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.note_buffer.len() {
            None
        } else {
            match Self::try_next_note(&mut self.offset, self.note_buffer) {
                Ok(note) => Some(note),
                Err(e) => {
                    error!("{}", e);
                    None
                }
            }
        }
    }
}

/// Helper function to iterate over ELF notes in a buffer.
pub fn iterate_elf_notes(note_buffer: &[u8]) -> ElfNoteIterator {
    ElfNoteIterator::new(note_buffer)
}

#[derive(Debug, PartialEq, Eq)]
#[repr(C)]
/// Time value for a process.
pub struct ProcessTimeVal {
    pub tv_sec: u64,
    pub tv_usec: u64,
}

#[derive(Debug, PartialEq, Eq)]
#[repr(C)]
/// Deserialized process status note.
///
/// This is the deserialized form of the NT_PRSTATUS note type.
/// Note that this structure is architecture-specific.
pub struct ProcessStatusNote {
    pub si_signo: u32,
    pub si_code: u32,
    pub si_errno: u32,
    pub pr_cursig: u16,
    pub pad0: u16,
    pub pr_sigpend: u64,
    pub pr_sighold: u64,
    pub pr_pid: u32,
    pub pr_ppid: u32,
    pub pr_pgrp: u32,
    pub pr_sid: u32,
    pub pr_utime: ProcessTimeVal,
    pub pr_stime: ProcessTimeVal,
    pub pr_cutime: ProcessTimeVal,
    pub pr_cstime: ProcessTimeVal,
    pub pr_reg: ElfGRegSet,
    pub pr_fpvalid: u32,
    pub pad1: u32,
}

impl<'a> TryFrom<&'a [u8]> for &'a ProcessStatusNote {
    type Error = Error;

    fn try_from(value: &'a [u8]) -> std::result::Result<Self, Self::Error> {
        if value.len() != size_of::<ProcessStatusNote>() {
            return Err(eyre!("Invalid size for ProcessStatusNote: {}", value.len()));
        }

        // SAFETY: ProcessStatusNote only contains scalar values, no pointers.
        unsafe { (value.as_ptr() as *const ProcessStatusNote).as_ref() }
            .ok_or(eyre!("Invalid pointer ProcessStatusNote"))
    }
}

/// An entry in a ElfNote::File::mapped_files vector.
#[derive(Debug, PartialEq, Eq)]
pub struct MappedFile<'a> {
    pub path: Option<&'a Path>,
    pub start_addr: usize,
    pub end_addr: usize,
    pub page_offset: usize,
}

#[derive(Debug, PartialEq, Eq)]
/// Parsed CORE::NT_FILE note.
pub struct FileNote<'a> {
    page_size: usize,
    mapped_files: Vec<MappedFile<'a>>,
    /// The input data was incomplete, so the mapped_files list is not complete.
    incomplete: bool,
}

impl<'a> FileNote<'a> {
    const NT_FILE_ENTRY_SIZE: usize = size_of::<ElfPtrSize>() * 3;

    /// Parses a CORE::NT_FILE note's description data.
    ///
    /// Really tries hard to parse out as much as possible, even if the data is incomplete.
    /// For example, if the string table is missing, it will still parse the header, but set the
    /// path to None.
    fn try_parse(data: &'a [u8]) -> Result<Self> {
        // See linux/fs/binfmt_elf.c:
        // https://github.com/torvalds/linux/blob/6465e260f48790807eef06b583b38ca9789b6072/fs/binfmt_elf.c#L1633-L1644
        //
        //  - long count     -- how many files are mapped
        //  - long page_size -- units for file_ofs
        //  - array of [COUNT] elements of
        //     - long start
        //     - long end
        //     - long file_ofs
        //  - followed by COUNT filenames in ASCII: "FILE1" NUL "FILE2" NUL...
        //
        let mut offset = 0;
        let count = data.gread::<ElfPtrSize>(&mut offset)? as usize;
        let page_size = data.gread::<ElfPtrSize>(&mut offset)? as usize;
        let mut mapped_files = Vec::with_capacity(count);

        let str_table_start = min(offset + count * Self::NT_FILE_ENTRY_SIZE, data.len());
        let str_table = &data[str_table_start..];
        let mut str_table_offset = 0;

        let mut incomplete = false;

        let mut get_next_path = || -> Option<&Path> {
            match str_table.gread::<&CStr>(&mut str_table_offset) {
                Ok(cstr) => Some(Path::new(OsStr::from_bytes(cstr.to_bytes()))),
                Err(_) => {
                    incomplete = true;
                    None
                }
            }
        };

        let mut parse_entry = || -> Result<MappedFile> {
            let start_addr = data.gread::<ElfPtrSize>(&mut offset)? as usize;
            let end_addr = data.gread::<ElfPtrSize>(&mut offset)? as usize;
            let page_offset = data.gread::<ElfPtrSize>(&mut offset)? as usize;
            Ok(MappedFile {
                path: get_next_path(),
                start_addr,
                end_addr,
                page_offset,
            })
        };

        for _ in 0..count {
            match parse_entry() {
                Ok(entry) => mapped_files.push(entry),
                Err(_) => {
                    incomplete = true;
                    break;
                }
            }
        }

        if incomplete {
            // Log an error but keep the list with what we've gathered so far
            warn!("Incomplete NT_FILE note.");
        }

        Ok(Self {
            page_size,
            incomplete,
            mapped_files,
        })
    }

    // TODO: MFLT-11766 Use NT_FILE note and PT_LOADs in case /proc/pid/maps read failed
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &MappedFile> {
        self.mapped_files.iter()
    }
}

impl<'a> TryFrom<&'a [u8]> for FileNote<'a> {
    type Error = Error;

    fn try_from(value: &'a [u8]) -> std::result::Result<Self, Self::Error> {
        Self::try_parse(value)
    }
}

#[cfg(test)]
mod test {
    use hex::decode;
    use insta::assert_debug_snapshot;
    use rstest::rstest;
    use scroll::IOwrite;
    use std::fs::File;
    use std::io::{Cursor, Read, Write};
    use std::path::PathBuf;

    use super::*;

    #[rstest]
    // Header-only size in case there are no name and description:
    #[case(
        "",
        0,
        "00000000\
         00000000\
         78563412"
    )]
    // Description data is padded to 4-byte alignment:
    #[case(
        "",
        1,
        "00000000\
         01000000\
         78563412\
         FF000000"
    )]
    // Description data already 4-byte aligned:
    #[case(
        "",
        4,
        "00000000\
         04000000\
         78563412\
         FFFFFFFF"
    )]
    // Name data and size includes NUL terminator and is padded to 4-byte alignment:
    #[case(
        "ABC",
        0,
        "04000000\
         00000000\
         78563412\
         41424300"
    )]
    // Both name and description:
    #[case(
        "A",
        1,
        "02000000\
         01000000\
         78563412\
         41000000\
         FF000000"
    )]
    fn test_build_elf_note(
        #[case] name: &str,
        #[case] description_size: usize,
        #[case] expected_buffer_contents_hex: &str,
    ) {
        let note_type = 0x12345678;
        let note_desc = [0xffu8; 40];
        let note = build_elf_note(name, &note_desc[..description_size], note_type).unwrap();

        // compare the note buffer contents to the expected hex string:
        let expected_buffer_contents = decode(expected_buffer_contents_hex).unwrap();
        assert_eq!(note, expected_buffer_contents);
    }

    #[test]
    fn test_elf_note_try_parse_build_id_note() {
        let build_id = b"ABCD";
        assert_eq!(
            ElfNote::try_parse(b"GNU", 3, build_id).unwrap().unwrap(),
            ElfNote::GnuBuildId(build_id)
        );
    }

    #[test]
    fn test_iterate_elf_notes_with_fixture() {
        let name = "sample_note";
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/cli/memfault_core_handler/fixtures")
            .join(name)
            .with_extension("bin");
        let mut input_file = File::open(input_path).unwrap();

        let mut note = Vec::new();
        input_file.read_to_end(&mut note).unwrap();

        let all_notes: Vec<_> = iterate_elf_notes(&note).collect();
        assert_eq!(all_notes.len(), 7);

        let notes: Vec<_> = all_notes
            .into_iter()
            .filter(|n| match n {
                ElfNote::ProcessStatus(_) => true,
                ElfNote::File { .. } => true,
                ElfNote::Auxv(_) => true,
                ElfNote::GnuBuildId { .. } => false, // Fixture doesn't have a build ID note
                ElfNote::Unknown { .. } => false,    // Ignore
            })
            .collect();

        assert_debug_snapshot!(notes);
    }

    #[test]
    fn test_iterate_elf_notes_empty() {
        let notes: Vec<_> = ElfNoteIterator::collect(iterate_elf_notes(&[]));
        assert!(notes.is_empty());
    }

    #[rstest]
    // Aligned desc:
    #[case(b"TestNote", &[1, 2, 3, 4, 5, 6, 7, 8])]
    // Non-aligned desc:
    #[case(b"TestNote", &[1, 2, 3, 4, 5])]
    // Empty desc:
    #[case(b"TestNote", &[])]
    // Aligned name:
    #[case(b"TestNot", &[])]
    // Empty name:
    #[case(b"", &[1, 2, 3, 4, 5])]
    fn test_iterate_elf_notes_basic_edge_cases(
        #[case] name_value: &[u8],
        #[case] note_desc: &[u8],
    ) {
        let note_type_value = 0x12345678;
        let note_buffer = build_elf_note(
            &String::from_utf8(name_value.into()).unwrap(),
            note_desc,
            note_type_value,
        )
        .unwrap();
        let notes: Vec<_> = iterate_elf_notes(note_buffer.as_slice()).collect();
        assert_eq!(notes.len(), 1);

        match &notes[0] {
            ElfNote::Unknown {
                name,
                note_type,
                description,
            } => {
                assert_eq!(*name, name_value);
                assert_eq!(*note_type, note_type_value);
                assert_eq!(description, &note_desc);
            }
            _ => panic!("Expected unknown note"),
        };
    }

    #[test]
    fn test_iterate_elf_notes_note_parsing_failure_handling() {
        // 2 NT_PRSTATUS notes with a description that is 1 byte short,
        // making ElfNote::try_parse fail and returning them as ElfNote::Unknown notes:
        let note_desc = [0xffu8; size_of::<ProcessStatusNote>() - 1];
        let note_buffer = [
            build_elf_note("CORE", &note_desc, NT_PRSTATUS).unwrap(),
            build_elf_note("CORE", &note_desc, NT_PRSTATUS).unwrap(),
        ]
        .concat();

        // Note: iteration continues, even if parsing fails for a note:
        let notes: Vec<_> = iterate_elf_notes(note_buffer.as_slice()).collect();
        assert_eq!(notes.len(), 2);
        for n in notes {
            assert!(matches!(n, ElfNote::Unknown { .. }));
        }
    }

    #[rstest]
    // Note header is too short:
    #[case(size_of::<Nhdr>() - 1)]
    // Name is short:
    #[case(size_of::<Nhdr>() + 1)]
    // Desc is short:
    #[case(size_of::<Nhdr>() + 8 /* "Hello" + padding */ + 1)]
    fn test_iterate_elf_notes_note_data_short(#[case] note_size: usize) {
        let note_type_value = 0x12345678;
        let note_decs = [0xffu8; 40];
        let note_buffer = build_elf_note("Hello", &note_decs, note_type_value).unwrap();

        // Note data is short -- iteration should stop immediately (no more data to read after):
        let notes: Vec<_> = ElfNoteIterator::collect(iterate_elf_notes(&note_buffer[..note_size]));
        assert!(notes.is_empty());
    }

    const NT_FILE_HDR_SIZE: usize = size_of::<ElfPtrSize>() * 2;

    #[rstest]
    // If we don't have the header, the parsing will fail:
    #[case(0)]
    #[case(NT_FILE_HDR_SIZE - 1)]
    fn test_elf_note_try_parse_file_note_too_short(#[case] desc_size: usize) {
        let file_note_desc = make_nt_file_note_data_fixture();

        // Clip the description data to the given size:
        let file_note_desc = &file_note_desc[..desc_size];

        assert!(FileNote::try_parse(file_note_desc).is_err());
    }

    #[rstest]
    // Header only -- no mapped_files:
    #[case(
        NT_FILE_HDR_SIZE,
        FileNote {
            mapped_files: vec![],
            incomplete: true,
            page_size: 0x1000,
        }
    )]
    // Header + one entry -- one mapped_file, no paths (missing string table):
    #[case(
        NT_FILE_HDR_SIZE + FileNote::NT_FILE_ENTRY_SIZE,
        FileNote {
            mapped_files: vec![
                MappedFile {
                    path: None,
                    start_addr: 0,
                    end_addr: 1,
                    page_offset: 2,
                },
            ],
            incomplete: true,
            page_size: 0x1000,
        }
    )]
    // Header + one entry -- one mapped_file, no paths (missing string table):
    #[case(
        NT_FILE_HDR_SIZE + (2 * FileNote::NT_FILE_ENTRY_SIZE) + 16,
        FileNote {
            mapped_files: vec![
                MappedFile {
                    path: Some(Path::new("/path/to/file")),
                    start_addr: 0,
                    end_addr: 1,
                    page_offset: 2,
                },
                MappedFile {
                    path: None,
                    start_addr: 0,
                    end_addr: 1,
                    page_offset: 2,
                },
            ],
            incomplete: true,
            page_size: 0x1000,
        }
    )]

    fn test_elf_note_try_parse_file_note_incomplete_string_table(
        #[case] desc_size: usize,
        #[case] expected: FileNote,
    ) {
        let file_note_desc = make_nt_file_note_data_fixture();

        // Clip the description data to the given size:
        let file_note_desc = &file_note_desc[..desc_size];

        let note = FileNote::try_parse(file_note_desc).unwrap();
        assert_eq!(note, expected);
    }

    const FIXTURE_FILE_PATH: &[u8; 14] = b"/path/to/file\0";

    fn make_nt_file_note_data_fixture() -> Vec<u8> {
        let mut cursor = Cursor::new(vec![]);
        let count = 2;

        // Header:
        // Count:
        cursor.iowrite::<ElfPtrSize>(count).unwrap();
        // Page size (0x1000):
        cursor.iowrite::<ElfPtrSize>(0x1000).unwrap();

        // Entries:
        for _ in 0..count {
            // Start, end, file offset:
            for n in 0..3 {
                cursor.iowrite::<ElfPtrSize>(n).unwrap();
            }
        }

        // String table:
        for _ in 0..count {
            let _ = cursor.write(FIXTURE_FILE_PATH).unwrap();
        }

        let file_note_desc = cursor.into_inner();
        assert_eq!(file_note_desc.len(), 92);
        file_note_desc
    }
}
