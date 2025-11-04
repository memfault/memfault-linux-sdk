//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::{self},
    path::{Path, PathBuf},
};

use eyre::{Context, Result};
use flate2::Compression;
use log::warn;

pub const DEFAULT_GZIP_COMPRESSION_LEVEL: Compression = Compression::new(4);

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

/// Move a directory. Try `fs::rename` first which is most efficient but only
/// works if source and destination are on the same filesystem.
/// Falls back to recursively copying the contents and removing the source.
/// Returns an error if `target` is inside `source` (which would otherwise
/// lead to strange behavior).
pub fn move_dir(source: &PathBuf, target: &PathBuf) -> Result<()> {
    // Prevent moving a directory into one of its own descendants.
    if target.starts_with(source) {
        return Err(eyre::eyre!(
            "Failed to move directory, target {} is inside source {}",
            target.display(),
            source.display()
        ));
    }

    if fs::rename(source, target).is_ok() {
        return Ok(());
    }

    copy_dir(source.as_path(), target.as_path())?;
    fs::remove_dir_all(source)
        .wrap_err_with(|| format!("Error removing source directory {}", source.display()))?;
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    use std::collections::VecDeque;

    if dst.starts_with(src) {
        return Err(eyre::eyre!(
            "Failed to copy directory, target {} is inside source {}",
            dst.display(),
            src.display()
        ));
    }

    let mut stack: VecDeque<(PathBuf, PathBuf)> = VecDeque::new();
    stack.push_back((src.to_path_buf(), dst.to_path_buf()));

    while let Some((cur_src, cur_dst)) = stack.pop_back() {
        fs::create_dir_all(&cur_dst)
            .wrap_err_with(|| format!("Error creating directory {}", cur_dst.display()))?;

        for entry in fs::read_dir(&cur_src)
            .wrap_err_with(|| format!("Error reading directory {}", cur_src.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let from = entry.path();
            let to = cur_dst.join(entry.file_name());

            if file_type.is_dir() {
                // Push directory for later processing
                stack.push_back((from, to));
            } else if file_type.is_file() {
                fs::copy(&from, &to).wrap_err_with(|| {
                    format!("Error copying file {} to {}", from.display(), to.display())
                })?;
            } else if file_type.is_symlink() {
                // Preserve symlinks on unix by recreating them. Otherwise copy.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    let link_target = fs::read_link(&from)?;
                    symlink(&link_target, &to)?;
                }
                #[cfg(not(unix))]
                {
                    fs::copy(&from, &to).wrap_err_with(|| {
                        format!(
                            "Error copying symlink {} to {}",
                            from.display(),
                            to.display()
                        )
                    })?;
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_move_dir_rename_success() -> Result<()> {
        let src_dir = tempdir()?;
        let dst_dir = tempdir()?;

        let src_path = src_dir.path().join("a");
        fs::create_dir(&src_path)?;
        let f = src_path.join("foo.txt");
        fs::write(&f, b"hello")?;

        let target = dst_dir.path().join("moved");

        // On the same filesystem rename should succeed
        move_dir(&src_path, &target)?;

        assert!(target.exists());
        assert!(target.join("foo.txt").exists());
        assert!(!src_path.exists());
        Ok(())
    }

    #[test]
    fn test_copy_dir_contents() -> Result<()> {
        let src_dir = tempdir()?;
        let dst_dir = tempdir()?;

        let src_path = src_dir.path().join("src");
        fs::create_dir(&src_path)?;
        fs::write(src_path.join("one.txt"), b"1")?;
        fs::write(src_path.join("two.txt"), b"2")?;

        let target = dst_dir.path().join("copied");

        // Call copy_dir directly to validate recursive copy behavior used by fallback.
        copy_dir(&src_path, &target)?;

        assert!(target.exists());
        assert!(target.join("one.txt").exists());
        assert!(target.join("two.txt").exists());
        Ok(())
    }

    #[test]
    fn test_move_dir_target_inside_source_is_error() {
        let src_dir = tempdir().unwrap();
        let src_path = src_dir.path().join("parent");
        fs::create_dir(&src_path).unwrap();
        let target = src_path.join("parent/child/target");

        let res = move_dir(&src_path, &target);
        assert!(res.is_err());
    }

    #[test]
    fn test_copy_dir_target_inside_source_is_error() {
        let src_dir = tempdir().unwrap();
        let src_path = src_dir.path().join("parent");
        fs::create_dir(&src_path).unwrap();
        let target = src_path.join("parent/child/target");

        let res = copy_dir(&src_path, &target);
        assert!(res.is_err());
    }
}
