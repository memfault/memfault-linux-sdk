//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    ffi::CString,
    fs::{read_dir, Metadata},
    mem,
    ops::{Add, AddAssign, Sub},
    os::unix::prelude::OsStrExt,
    path::Path,
};

use eyre::{eyre, Result};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// Disk space information in bytes and inodes.
pub struct DiskSize {
    /// Bytes on disk
    pub bytes: u64,
    /// Number of inodes
    pub inodes: u64,
}

impl DiskSize {
    pub fn new_capacity(bytes: u64) -> Self {
        Self {
            bytes,
            inodes: u64::MAX,
        }
    }

    pub const ZERO: Self = Self {
        bytes: 0,
        inodes: 0,
    };

    pub fn min(a: Self, b: Self) -> Self {
        Self {
            bytes: a.bytes.min(b.bytes),
            inodes: a.inodes.min(b.inodes),
        }
    }

    pub fn exceeds(&self, other: &Self) -> bool {
        (self.bytes != other.bytes || self.inodes != other.inodes)
            && self.bytes >= other.bytes
            && self.inodes >= other.inodes
    }
}

impl Add for DiskSize {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            bytes: self.bytes + other.bytes,
            inodes: self.inodes + other.inodes,
        }
    }
}

impl AddAssign for DiskSize {
    fn add_assign(&mut self, rhs: Self) {
        self.bytes += rhs.bytes;
        self.inodes += rhs.inodes;
    }
}

impl Sub for DiskSize {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            bytes: self.bytes.saturating_sub(other.bytes),
            inodes: self.inodes.saturating_sub(other.inodes),
        }
    }
}

impl From<Metadata> for DiskSize {
    fn from(metadata: Metadata) -> Self {
        Self {
            bytes: metadata.len(),
            inodes: 1,
        }
    }
}

// We need to cast to u64 here on some platforms.
#[allow(clippy::unnecessary_cast)]
pub fn get_disk_space(path: &Path) -> Result<DiskSize> {
    let mut stat: libc::statvfs = unsafe { mem::zeroed() };
    let cpath = CString::new(path.as_os_str().as_bytes()).map_err(|_| eyre!("Invalid path"))?;
    // danburkert/fs2-rs#1: cast is necessary for platforms where c_char != u8.
    if unsafe { libc::statvfs(cpath.as_ptr() as *const _, &mut stat) } != 0 {
        Err(eyre!("Unable to call statvfs"))
    } else {
        Ok(DiskSize {
            // Note that we use f_bavail/f_favail instead of f_bfree/f_bavail.
            // f_bfree is the number of free blocks available to the
            // superuser, but we want to stop before getting to that
            // point. [bf]avail is what is available to normal users.
            bytes: stat.f_frsize as u64 * stat.f_bavail as u64,
            inodes: stat.f_favail as u64,
        })
    }
}

/// fs_extra::get_size but also returning the number of inodes
pub fn get_size<P>(path: P) -> Result<DiskSize>
where
    P: AsRef<Path>,
{
    // Using `fs::symlink_metadata` since we don't want to follow symlinks,
    // as we're calculating the exact size of the requested path itself.
    let path_metadata = path.as_ref().symlink_metadata()?;

    let mut size = DiskSize::ZERO;

    if path_metadata.is_dir() {
        for entry in read_dir(&path)? {
            let entry = entry?;
            // `DirEntry::metadata` does not follow symlinks (unlike `fs::metadata`), so in the
            // case of symlinks, this is the size of the symlink itself, not its target.
            let entry_metadata = entry.metadata()?;

            if entry_metadata.is_dir() {
                // The size of the directory entry itself will be counted inside the `get_size()` call,
                // so we intentionally don't also add `entry_metadata.len()` to the total here.
                size += get_size(entry.path())?;
            } else {
                size += entry_metadata.into();
            }
        }
    } else {
        size = path_metadata.into();
    }

    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(0, 0, 0, 0, false)]
    // When bytes and inodes are greater
    #[case(1024, 1, 0, 0, true)]
    // When bytes and inodes are lesser
    #[case(1024, 10, 2048, 20, false)]
    // When bytes or inodes are greater
    #[case(1024, 100, 2048, 20, false)]
    #[case(4096, 10, 2048, 20, false)]
    // When bytes are equal and inodes are greater
    #[case(1024, 100, 1024, 10, true)]
    fn test_size_cmp(
        #[case] bytes: u64,
        #[case] inodes: u64,
        #[case] free_bytes: u64,
        #[case] free_inodes: u64,

        #[case] exceeds: bool,
    ) {
        let size1 = DiskSize { bytes, inodes };
        let size2 = DiskSize {
            bytes: free_bytes,
            inodes: free_inodes,
        };
        assert_eq!(size1.exceeds(&size2), exceeds);
    }
}
