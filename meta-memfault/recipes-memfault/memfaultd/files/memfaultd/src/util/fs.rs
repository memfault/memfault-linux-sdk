//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::{self},
    path::{Path, PathBuf},
};

use eyre::{Context, Result};
use log::warn;

/// Takes a directory and returns a vector of all files in that directory, sorted
/// by creation date:
#[allow(dead_code)] // Required to build without warnings and --no-default-features
pub fn get_files_sorted_by_mtime(dir: &Path) -> Result<Vec<PathBuf>> {
    let read_dir = std::fs::read_dir(dir)?;
    let mut entries = read_dir
        .filter_map(|e| match e {
            Ok(e) => Some(e),
            Err(e) => {
                warn!("Error reading directory entry: {:#}", e);
                None
            }
        })
        .filter(|entry| entry.path().is_file())
        .collect::<Vec<_>>();
    // Order by oldest first:
    entries.sort_by_key(|entry| {
        entry
            .metadata()
            .map(|m| m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    Ok(entries.into_iter().map(|m| m.path()).collect())
}

/// Move a file. Try fs::rename first which is most efficient but only works if source
/// and destination are on the same filesystem.
/// Use Copy/Delete strategy if rename failed.
pub fn move_file(source: &PathBuf, target: &PathBuf) -> Result<()> {
    if fs::rename(source, target).is_err() {
        fs::copy(source, target).wrap_err_with(|| {
            format!(
                "Error moving file {} to {}",
                source.display(),
                target.display()
            )
        })?;
        fs::remove_file(source)?;
    }
    Ok(())
}

/// Copy a file.
///
/// If the source and target are the same, do nothing.
pub fn copy_file(source: &PathBuf, target: &PathBuf) -> Result<()> {
    if source == target {
        return Ok(());
    }

    fs::copy(source, target).wrap_err_with(|| {
        format!(
            "Error copying file {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}
