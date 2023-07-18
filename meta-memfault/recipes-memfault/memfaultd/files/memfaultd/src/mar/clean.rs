//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{
    mar::MarEntry,
    util::disk_size::{get_disk_space, get_size, DiskSize},
};
use eyre::{eyre, Result, WrapErr};
use log::{debug, trace, warn};
use std::fs::read_dir;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub struct MarStagingCleaner {
    mar_staging_path: PathBuf,
    max_total_size: DiskSize,
    min_headroom: DiskSize,
}

impl MarStagingCleaner {
    pub fn new(mar_staging_path: &Path, max_total_size: DiskSize, min_headroom: DiskSize) -> Self {
        Self {
            mar_staging_path: mar_staging_path.to_owned(),
            max_total_size,
            min_headroom,
        }
    }

    /// Cleans up MAR entries in the staging folder.
    /// Returns the amount of space left until the max_total_size will be reached or until
    /// min_headroom will be exceeded (the smallest of the two values).
    pub fn clean(&self, required_space: DiskSize) -> Result<DiskSize> {
        trace!("Cleaning MAR staging area...");
        clean_mar_staging(
            &self.mar_staging_path,
            self.max_total_size - required_space,
            get_disk_space(&self.mar_staging_path).unwrap_or(DiskSize::ZERO),
            self.min_headroom + required_space,
            SystemTime::now(),
        )
        .map_err(|e| {
            warn!("Unable to clean MAR entries: {}", e);
            e
        })
    }
}

fn clean_mar_staging(
    mar_staging: &Path,
    max_total_size: DiskSize,
    mut available_space: DiskSize,
    min_space: DiskSize,
    reference_date: SystemTime,
) -> Result<DiskSize> {
    #[derive(Debug)]
    struct AgeSizePath {
        age: Duration,
        size: DiskSize,
        path: PathBuf,
    }

    impl AgeSizePath {
        fn new(path: &Path, timestamp: SystemTime, reference_date: SystemTime) -> AgeSizePath {
            Self {
                age: (reference_date.duration_since(timestamp)).unwrap_or(Duration::ZERO),
                size: get_size(path).unwrap_or(DiskSize::ZERO),
                path: path.to_owned(),
            }
        }
    }

    let mut entries: Vec<AgeSizePath> = read_dir(mar_staging)
        .wrap_err(eyre!(
            "Unable to open MAR staging area: {}",
            mar_staging.display()
        ))?
        .filter_map(|r| r.map_err(|e| warn!("Unable to read DirEntry: {}", e)).ok())
        .map(|dir_entry| match MarEntry::from_path(dir_entry.path()) {
            // Use the collection time from the manifest or folder creation time if the manifest cannot be parsed:
            Ok(entry) => AgeSizePath::new(
                &entry.path,
                entry.manifest.collection_time.timestamp.into(),
                reference_date,
            ),
            Err(_) => {
                let path = dir_entry.path();
                let timestamp = path
                    .metadata()
                    .and_then(|m| m.created())
                    .unwrap_or_else(|_| SystemTime::now());
                AgeSizePath::new(&path, timestamp, reference_date)
            }
        })
        .collect();

    // Sort the entries by age, newest first:
    entries.sort_by(|a, b| b.age.cmp(&a.age));

    let mut total_size = DiskSize::ZERO;
    for entry in entries {
        // Note that (total_size + entry.size).exceeds(max_total_size) would not
        // work here because it would be false when only one of bytes/inode is
        // exceeded. (we have a test to verify this)
        let max_total_size_exceeded = !max_total_size.exceeds(&(total_size + entry.size));
        // Same here, min_space.exceeds(available_space) is sufficient but not necessary.
        let min_headroom_exceeded = !available_space.exceeds(&min_space);

        if max_total_size_exceeded || min_headroom_exceeded {
            debug!(
                "Cleaning up MAR entry: {} ({} bytes / {} inodes, ~{} days old, reasons: total={}, headroom={})",
                entry.path.display(),
                entry.size.bytes,
                entry.size.inodes,
                entry.age.as_secs() / (60 * 60 * 24),
                max_total_size_exceeded,
                min_headroom_exceeded,
            );
            if let Err(e) = std::fs::remove_dir_all(&entry.path) {
                warn!("Unable to remove MAR entry: {}", e);
                // If we can't remove the entry, we still want to count its size towards the total:
                total_size += get_size(&entry.path).unwrap_or(entry.size);
            } else {
                // Update the available space with an estimate of how much space was reclaimed by
                // deleting the entry. Note in reality this is (likely) more due to the space
                // occupied by inodes.
                debug!(
                    "Removed MAR entry: {} {:?}",
                    entry.path.display(),
                    entry.size
                );
                available_space += entry.size;
            }
        } else {
            total_size += entry.size;
        }
    }
    let remaining_quota = max_total_size - total_size;
    let usable_space = available_space - min_space;

    // Available space to write is the min of bytes and inodes remaining.
    Ok(DiskSize::min(remaining_quota, usable_space))
}

#[cfg(test)]
mod test {
    use crate::mar::test_utils::MarCollectorFixture;
    use crate::test_utils::create_file_with_size;
    use crate::test_utils::setup_logger;
    use rstest::{fixture, rstest};

    use super::*;

    #[rstest]
    fn empty_staging_area(
        mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
    ) {
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space - min_headroom)
        );
    }

    #[rstest]
    fn keeps_recent_unfinished_mar_entry(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
    ) {
        let path = mar_fixture.create_empty_entry();
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space - min_headroom)
        );
        assert!(path.exists());
    }

    #[rstest]
    fn removes_unfinished_mar_entry_exceeding_max_total_size(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
    ) {
        let path = mar_fixture.create_empty_entry();

        create_file_with_size(&path.join("log.txt"), max_total_size.bytes + 1).unwrap();
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space
                - DiskSize {
                    bytes: 0,
                    inodes: 1,
                },
            min_headroom,
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space - min_headroom)
        );
        assert!(!path.exists());
    }

    #[rstest]
    fn keeps_recent_mar_entry(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
    ) {
        let now = SystemTime::now();
        let path = mar_fixture.create_logentry_with_size_and_age(1, now);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
        )
        .unwrap();
        assert!(max_total_size.exceeds(&size_avail));
        assert!(path.exists());
    }

    #[rstest]
    fn removes_mar_entry_exceeding_max_total_size(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
    ) {
        let now = SystemTime::now();
        // NOTE: the entire directory will be larger than max_total_size due to the manifest.json.
        let path = mar_fixture.create_logentry_with_size_and_age(max_total_size.bytes, now);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space
                - DiskSize {
                    bytes: 0,
                    inodes: 2,
                },
            min_headroom,
            now,
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space - min_headroom)
        );
        assert!(!path.exists());
    }

    #[rstest]
    fn removes_mar_entry_exceeding_min_headroom(
        _setup_logger: (),
        mut mar_fixture: MarCollectorFixture,
    ) {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(4096);
        let min_headroom = DiskSize {
            bytes: 1024,
            inodes: 10,
        };
        let available_space = DiskSize {
            bytes: min_headroom.bytes - 1,
            inodes: 100,
        };
        // NOTE: the entire directory will be larger than 1 byte due to the manifest.json.
        let path = mar_fixture.create_logentry_with_size_and_age(1, now);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(!path.exists());
    }

    #[rstest]
    fn removes_mar_entry_exceeding_min_headroom_inodes(
        _setup_logger: (),
        mut mar_fixture: MarCollectorFixture,
    ) {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(10 * 1024 * 1024);
        let min_headroom = DiskSize {
            bytes: 1024,
            inodes: 10,
        };
        let available_space = DiskSize {
            bytes: max_total_size.bytes,
            inodes: 5,
        };
        // NOTE: the entire directory will be larger than 1 byte due to the manifest.json.
        let path = mar_fixture.create_logentry_with_size_and_age(1, now);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(!path.exists());
    }

    #[fixture]
    fn max_total_size() -> DiskSize {
        DiskSize::new_capacity(1024)
    }

    #[fixture]
    fn available_space() -> DiskSize {
        DiskSize {
            bytes: u64::MAX / 2,
            inodes: u64::MAX / 2,
        }
    }

    #[fixture]
    fn min_headroom() -> DiskSize {
        DiskSize {
            bytes: 0,
            inodes: 0,
        }
    }

    #[fixture]
    fn mar_fixture() -> MarCollectorFixture {
        MarCollectorFixture::new()
    }
}
