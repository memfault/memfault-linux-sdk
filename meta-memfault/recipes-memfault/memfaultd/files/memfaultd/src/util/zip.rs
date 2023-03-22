//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::File;
use std::io::Cursor;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::str::from_utf8;

use crate::util::io::StreamLen;
use eyre::Result;
use flate2::CrcReader;
use take_mut::take;

/// Minimalistic zip encoder which generates a compression-less zip stream on the fly from a list of
/// files. The benefit of this is that it requires no temporary file storage. It implements the
/// std::io::Read trait, so it can be used with any std::io::Read consumer. It can also tell the
/// length of the stream beforehand, only looking at the list of files and their sizes on disk.
/// This is useful for example when needing to specify a Content-Length header for a HTTP request.
/// Note it is very minimalistic in its implementation: it only supports "store" (no compression).
/// It only supports the 32-bit zip format, so it is limited to max. 4GB file sizes and does not
/// allow for more than 65,535 entries. File timestamps are not implemented and neither are UTF-8
/// filenames.
/// Note that read() calls can copy less than the size of the caller's buffer, due to an
/// implementation detail. Therefore it's recommended to use std::io::BufReader to wrap this stream.

// Some implementation notes:
// - The zip format is described here: https://en.wikipedia.org/wiki/ZIP_(file_format)
// - For each file in the zip file, the contents of the file is prepended with a "local file header"
//   and suffixed with a "data descriptor". The local file header contains some metadata of the file
//   that follows and the data descriptor contains additional metadata that is conveniently gathered
//   while reading/writing the contents, like the CRC32 checksum of the file's contents.
// - After all files have been written, a "central directory" is written, which contains all the
//   metadata of the files again, but in a slightly different, more elaborate format. This is used
//   by the decoder/unarchiver to quickly access the list of files in the zip file.

pub struct ZipEncoder {
    files: Vec<ZipEntryInfo>,
    state: ZipEncoderState,
    bytes_written: usize,
}

enum ZipEncoderState {
    Init,
    LocalFiles {
        index: usize,
        reader: LocalFileReader,
    },
    CentralDirectory {
        index: usize,
        start_offset: usize,
        reader: Cursor<Vec<u8>>,
    },
    EndOfCentralDirectory {
        reader: Cursor<Vec<u8>>,
    },
    Done,
}

impl ZipEncoder {
    /// Creates a new ZipEncoder from a list of source files that should be included in the zip.
    pub fn new(files: Vec<ZipEntryInfo>) -> Self {
        Self {
            files,
            state: ZipEncoderState::Init,
            bytes_written: 0,
        }
    }

    fn new_local_files_state(&self, index: usize) -> std::io::Result<ZipEncoderState> {
        Ok(ZipEncoderState::LocalFiles {
            index,
            reader: LocalFileReader::new(&self.files[index])?,
        })
    }

    fn new_central_directory_state(&self, index: usize, start_offset: usize) -> ZipEncoderState {
        ZipEncoderState::CentralDirectory {
            index,
            start_offset,
            reader: Cursor::new(make_file_header(
                &self.files[index],
                FileHeaderKind::CentralDirectory,
            )),
        }
    }

    fn new_end_of_central_directory_state(&self, start_offset: usize) -> ZipEncoderState {
        let num_files = self.files.len();
        ZipEncoderState::EndOfCentralDirectory {
            reader: Cursor::new(make_end_of_central_directory(
                num_files as u16,
                (self.bytes_written - start_offset) as u32,
                start_offset as u32,
            )),
        }
    }

    #[cfg(test)]
    pub fn file_names(&self) -> Vec<&str> {
        self.files
            .iter()
            .map(|f| from_utf8(f.name.as_slice()).unwrap())
            .collect()
    }
}

impl StreamLen for ZipEncoder {
    /// Length of the zip stream in bytes.
    fn stream_len(&self) -> u64 {
        zip_stream_len(&self.files) as u64
    }
}

impl Read for ZipEncoder {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let reader: &mut dyn Read = match &mut self.state {
                ZipEncoderState::Init => {
                    if self.files.is_empty() {
                        self.state = self.new_end_of_central_directory_state(0);
                    } else {
                        self.state = self.new_local_files_state(0)?;
                    }
                    continue;
                }
                ZipEncoderState::LocalFiles { reader, .. } => reader,
                ZipEncoderState::CentralDirectory { reader, .. } => reader,
                ZipEncoderState::EndOfCentralDirectory { reader } => reader,
                ZipEncoderState::Done => {
                    return Ok(0);
                }
            };
            let n = reader.read(buf)?;
            self.bytes_written += n;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            self.state = match &self.state {
                ZipEncoderState::Init => unreachable!(),
                ZipEncoderState::LocalFiles { index, reader } => {
                    if let Some(crc) = reader.crc() {
                        self.files[*index].crc = crc;
                        let next_index = *index + 1;
                        if next_index < self.files.len() {
                            self.files[next_index].offset = self.bytes_written as u32;
                            self.new_local_files_state(next_index)?
                        } else {
                            self.new_central_directory_state(0, self.bytes_written)
                        }
                    } else {
                        // Will never panic because the above read() call returned Ok(0), which
                        // means the LocalFileReader is in the LocalFileReaderState::Done state.
                        unreachable!()
                    }
                }
                ZipEncoderState::CentralDirectory {
                    index,
                    start_offset,
                    ..
                } => {
                    let next_index = *index + 1;
                    if next_index < self.files.len() {
                        self.new_central_directory_state(next_index, *start_offset)
                    } else {
                        self.new_end_of_central_directory_state(*start_offset)
                    }
                }
                ZipEncoderState::EndOfCentralDirectory { .. } => ZipEncoderState::Done,
                ZipEncoderState::Done => unreachable!(),
            }
        }
    }
}

pub struct ZipEntryInfo {
    path: PathBuf,
    name: Vec<u8>,
    size: u64,
    /// Offset from the start of the file to the local file header.
    /// Gets filled in by ZipEncoder before it reads the file.
    offset: u32,
    // Checksum of the (uncompressed) file data.
    // Gets filled in by ZipEncoder after the file is read.
    crc: u32,
}

impl ZipEntryInfo {
    /// Creates a new ZipEntryInfo from a path to a file.
    /// The path must be relative to the base path. The base path is used to determine the name of
    /// the file in the zip, by stripping the base path from the file path.
    pub fn new(path: PathBuf, base: &Path) -> Result<Self> {
        let name = path.strip_prefix(base)?.as_os_str().as_bytes().to_owned();
        let metadata = path.metadata()?;
        Ok(Self {
            path,
            name,
            size: metadata.len(),
            crc: 0,
            offset: 0,
        })
    }
}

#[derive(Clone, Copy)]
enum FileHeaderKind {
    Local,
    CentralDirectory,
}

pub fn zip_stream_len_empty() -> usize {
    END_OF_CENTRAL_DIRECTORY_SIZE
}

/// Returns the size of the entire zip stream in bytes, given a slice of ZipEntryInfo.
pub fn zip_stream_len(files: &[ZipEntryInfo]) -> usize {
    files.iter().map(zip_stream_len_for_file).sum::<usize>() + zip_stream_len_empty()
}

/// Returns the size that a single ZipEntryInfo contributes to the size of the zip stream.
pub fn zip_stream_len_for_file(file_info: &ZipEntryInfo) -> usize {
    header_size(file_info, FileHeaderKind::Local)
        + file_info.size as usize
        + DATA_DESCRIPTOR_SIZE
        + header_size(file_info, FileHeaderKind::CentralDirectory)
}

fn header_size(info: &ZipEntryInfo, kind: FileHeaderKind) -> usize {
    const LOCAL_FILE_HEADER_SIZE: usize = 30;
    const DIRECTORY_HEADER_SIZE: usize = 46;
    let name_len = info.name.len();
    match kind {
        FileHeaderKind::Local => LOCAL_FILE_HEADER_SIZE + name_len,
        FileHeaderKind::CentralDirectory => DIRECTORY_HEADER_SIZE + name_len,
    }
}

fn make_file_header(info: &ZipEntryInfo, kind: FileHeaderKind) -> Vec<u8> {
    let mut header = Vec::with_capacity(header_size(info, kind));
    header.extend_from_slice(match &kind {
        FileHeaderKind::Local => {
            // Signature
            // Version needed to extract
            // General purpose bit flag (data descriptor enabled)
            // Compression mode (store / no compression)
            // File last modified time (all zeroes)
            // File last modified date (all zeroes)
            b"PK\x03\x04\
            \x0A\x00\
            \x08\x00\
            \x00\x00\
            \x00\x00\
            \x00\x00\
            "
        }
        FileHeaderKind::CentralDirectory => {
            // Signature
            // Version made by
            // Version needed to extract
            // General purpose bit flag (data descriptor enabled)
            // Compression mode (store / no compression)
            // File last modified time (all zeroes)
            // File last modified date (all zeroes)
            b"PK\x01\x02\
            \x0A\x00\
            \x0A\x00\
            \x08\x00\
            \x00\x00\
            \x00\x00\
            \x00\x00\
            "
        }
    });
    // CRC-32 of uncompressed data:
    header.extend_from_slice(&info.crc.to_le_bytes());
    let size_slice = &(info.size as u32).to_le_bytes();
    // Compressed size:
    header.extend_from_slice(size_slice);
    // Uncompressed size:
    header.extend_from_slice(size_slice);
    // File name length:
    header.extend_from_slice(&(info.name.len() as u16).to_le_bytes());
    // Extra field length (0 bytes):
    header.extend_from_slice(b"\x00\x00");

    if let FileHeaderKind::CentralDirectory = &kind {
        // File comment length (0 bytes)
        // Disk number where file starts (0)
        // Internal file attributes (0)
        // External file attributes (0)
        header.extend_from_slice(
            b"\x00\x00\
            \x00\x00\
            \x00\x00\
            \x00\x00\x00\x00\
            ",
        );

        // Relative offset of local file header:
        header.extend_from_slice(&info.offset.to_le_bytes());
    };

    // File name:
    header.extend_from_slice(&info.name);
    header
}

const DATA_DESCRIPTOR_SIZE: usize = 16;

fn make_data_descriptor(crc: u32, size: u32) -> Vec<u8> {
    let mut desc = Vec::with_capacity(DATA_DESCRIPTOR_SIZE);
    // Signature:
    desc.extend_from_slice(b"PK\x07\x08");
    // CRC-32 of uncompressed data:
    desc.extend_from_slice(&crc.to_le_bytes());
    let size_slice = &size.to_le_bytes();
    // Compressed size:
    desc.extend_from_slice(size_slice);
    // Uncompressed size:
    desc.extend_from_slice(size_slice);
    desc
}

const END_OF_CENTRAL_DIRECTORY_SIZE: usize = 22;

fn make_end_of_central_directory(num_files: u16, size: u32, offset: u32) -> Vec<u8> {
    let mut desc = Vec::with_capacity(END_OF_CENTRAL_DIRECTORY_SIZE);
    desc.extend_from_slice(
        // Signature
        // Number of this disk
        // Disk where central directory starts
        b"PK\x05\x06\
        \x00\x00\
        \x00\x00\
        ",
    );
    let num_files_slice = &num_files.to_le_bytes();
    // Number of central directory records on this disk:
    desc.extend_from_slice(num_files_slice);
    // Total number of central directory records
    desc.extend_from_slice(num_files_slice);
    // Size of central directory
    desc.extend_from_slice(&size.to_le_bytes());
    // Offset of start of central directory
    desc.extend_from_slice(&offset.to_le_bytes());
    // Comment length
    desc.extend_from_slice(b"\x00\x00");
    desc
}

struct LocalFileReader {
    state: LocalFileReaderState,
}

enum LocalFileReaderState {
    Header { reader: Cursor<Vec<u8>>, file: File },
    Data { reader: CrcReader<File> },
    DataDescriptor { reader: Cursor<Vec<u8>>, crc: u32 },
    Done { crc: u32 },
}

impl LocalFileReader {
    pub fn new(info: &ZipEntryInfo) -> std::io::Result<Self> {
        Ok(Self {
            state: LocalFileReaderState::Header {
                reader: Cursor::new(make_file_header(info, FileHeaderKind::Local)),
                file: File::open(&info.path)?,
            },
        })
    }

    pub fn crc(&self) -> Option<u32> {
        match self.state {
            LocalFileReaderState::Done { crc } => Some(crc),
            _ => None,
        }
    }
}

impl Read for LocalFileReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let reader: &mut dyn Read = match &mut self.state {
                LocalFileReaderState::Header { reader, .. } => reader,
                LocalFileReaderState::Data { reader } => reader,
                LocalFileReaderState::DataDescriptor { reader, .. } => reader,
                LocalFileReaderState::Done { .. } => {
                    return Ok(0);
                }
            };
            let n = reader.read(buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            take(&mut self.state, |state| match state {
                LocalFileReaderState::Header { file, .. } => LocalFileReaderState::Data {
                    reader: CrcReader::new(file),
                },
                LocalFileReaderState::Data { reader: crc_reader } => {
                    let crc = crc_reader.crc().sum();
                    LocalFileReaderState::DataDescriptor {
                        reader: Cursor::new(make_data_descriptor(crc, crc_reader.crc().amount())),
                        crc,
                    }
                }
                LocalFileReaderState::DataDescriptor { crc, .. } => {
                    LocalFileReaderState::Done { crc }
                }
                LocalFileReaderState::Done { .. } => unreachable!(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{copy, Cursor};
    use std::io::{Seek, SeekFrom};

    use crate::test_utils::create_file_with_contents;
    use tempfile::tempdir;
    use zip::ZipArchive;

    use super::*;

    /// Makes an empty zip file, then reads it with a "known good" zip unarchiver ('zip' crate).
    #[test]
    fn test_empty() {
        let (zip, zip_encoder) = zip_round_trip(vec![]);
        assert!(zip.is_empty());

        let stream_len = zip.into_inner().into_inner().len();
        assert_eq!(stream_len as u64, zip_encoder.stream_len());
    }

    /// Makes a zip with some files, then reads it with a "known good" zip unarchiver ('zip' crate).
    #[test]
    fn test_basic() {
        let tmp = tempdir().unwrap();
        let tempdir_path = tmp.path();

        let filenames_and_contents = vec![("hello.txt", "Hello World"), ("bye.txt", "Goodbye")];

        for (filename, contents) in filenames_and_contents.iter() {
            let file_path = tempdir_path.join(filename);
            create_file_with_contents(&file_path, contents.as_bytes()).unwrap();
        }

        let file_infos = filenames_and_contents
            .iter()
            .map(|(filename, _)| {
                let file_path = tempdir_path.join(filename);
                ZipEntryInfo::new(file_path, tempdir_path).unwrap()
            })
            .collect::<Vec<ZipEntryInfo>>();

        let (mut zip, zip_encoder) = zip_round_trip(file_infos);
        assert_eq!(zip.len(), filenames_and_contents.len());

        for (filename, contents) in filenames_and_contents.iter() {
            let vec: Vec<u8> = Vec::with_capacity(1024);
            let mut cursor = Cursor::new(vec);
            copy(&mut zip.by_name(filename).unwrap(), &mut cursor).unwrap();
            assert_eq!(cursor.into_inner(), contents.as_bytes());
        }

        let stream_len = zip.into_inner().into_inner().len();
        assert_eq!(stream_len as u64, zip_encoder.stream_len());
    }

    fn zip_round_trip(
        source_files: Vec<ZipEntryInfo>,
    ) -> (ZipArchive<Cursor<Vec<u8>>>, ZipEncoder) {
        let mut zip_encoder = ZipEncoder::new(source_files);
        let vec: Vec<u8> = Vec::with_capacity(1024 * 8);
        let mut cursor = Cursor::new(vec);
        copy(&mut zip_encoder, &mut cursor).unwrap();

        // Use flate2's ZipArchive to read the zip file we just created:
        cursor.seek(SeekFrom::Start(0)).unwrap();
        let zip = ZipArchive::new(cursor).unwrap();
        (zip, zip_encoder)
    }
}
