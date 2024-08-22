//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::{DirEntry, File},
    io::{Read, Seek, Write},
    path::Path,
};
use zip::{write::SimpleFileOptions, ZipWriter};

use eyre::{ContextCompat, Result};

/// Compress
pub fn zip_dir<T>(
    it: &mut dyn Iterator<Item = DirEntry>,
    prefix: &Path,
    writer: T,
    method: zip::CompressionMethod,
) -> Result<()>
where
    T: Write + Seek,
{
    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(0o755);

    let prefix = Path::new(prefix);
    let mut buffer = Vec::new();
    for entry in it {
        let path = entry.path();
        let name = path.strip_prefix(prefix)?;
        let path_as_string = name
            .to_str()
            .map(str::to_owned)
            .wrap_err_with(|| format!("{name:?} Is a Non UTF-8 Path"))?;

        // Write file or directory explicitly
        // Some unzip tools unzip files with directory paths correctly, some do not!
        if path.is_file() {
            zip.start_file(path_as_string, options)?;
            let mut f = File::open(path)?;

            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
            buffer.clear();
        } else if !name.as_os_str().is_empty() {
            // Only if not root! Avoids path spec / warning
            // and mapname conversion failed error on unzip
            zip.add_directory(path_as_string, options)?;
        }
    }
    zip.finish()?;
    Ok(())
}
