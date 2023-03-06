//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::mar::mar_entry::MarEntry;
use eyre::{eyre, Result, WrapErr};
use fs_extra::dir::get_size;
use log::{debug, trace, warn};
use std::fs::read_dir;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub struct MarStagingCleaner {
    mar_staging_path: PathBuf,
    max_total_size: u64,
}

impl MarStagingCleaner {
    pub fn new(mar_staging_path: &Path, max_total_size: u64) -> Self {
        Self {
            mar_staging_path: mar_staging_path.to_owned(),
            max_total_size,
        }
    }

    pub fn clean(&self, required_space_bytes: u64) -> Result<u64> {
        trace!("Cleaning MAR staging area...");
        clean_mar_staging(
            &self.mar_staging_path,
            self.max_total_size.saturating_sub(required_space_bytes),
            SystemTime::now(),
        )
        .map_err(|e| {
            warn!("Unable to clean MAR entries: {}", e);
            e
        })
    }
}

/// Clean up MAR entries in the staging folder, given an iterator to the entries.
/// Returns the amount of space left until the max_total_size will be reached.
fn clean_mar_staging(
    mar_staging: &Path,
    max_total_size: u64,
    reference_date: SystemTime,
) -> Result<u64> {
    struct AgeSizePath {
        age: Duration,
        size: u64,
        path: PathBuf,
    }

    impl AgeSizePath {
        fn new(path: &Path, timestamp: SystemTime, reference_date: SystemTime) -> AgeSizePath {
            Self {
                age: (reference_date.duration_since(timestamp)).unwrap_or(Duration::ZERO),
                size: get_size(path).unwrap_or(0),
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

    let mut total_size = 0;
    for entry in entries {
        if (total_size + entry.size) > max_total_size {
            debug!(
                "Cleaning up MAR entry: {} ({} bytes, ~{} days old)",
                entry.path.display(),
                entry.size,
                entry.age.as_secs() / (60 * 60 * 24)
            );
            if let Err(e) = std::fs::remove_dir_all(&entry.path) {
                warn!("Unable to remove MAR entry: {}", e);
                // If we can't remove the entry, we still want to count its size towards the total:
                total_size += get_size(&entry.path).unwrap_or(entry.size);
            }
        } else {
            total_size += entry.size;
        }
    }

    Ok(max_total_size - total_size)
}

#[cfg(test)]
mod test {
    use crate::mar::test_utils::MarCollectorFixture;
    use crate::test_utils::create_file_with_size;
    use rstest::{fixture, rstest};

    use super::*;

    #[rstest]
    fn empty_staging_area(mar_fixture: MarCollectorFixture, max_total_size: u64) {
        let size_avail =
            clean_mar_staging(&mar_fixture.mar_staging, max_total_size, SystemTime::now()).unwrap();
        assert_eq!(size_avail, max_total_size);
    }

    #[rstest]
    fn keeps_recent_unfinished_mar_entry(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: u64,
    ) {
        let path = mar_fixture.create_empty_entry();
        let size_avail =
            clean_mar_staging(&mar_fixture.mar_staging, max_total_size, SystemTime::now()).unwrap();
        assert_eq!(size_avail, max_total_size);
        assert!(path.exists());
    }

    #[rstest]
    fn removes_too_large_unfinished_mar_entry(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: u64,
    ) {
        let path = mar_fixture.create_empty_entry();
        create_file_with_size(&path.join("log.txt"), max_total_size + 1).unwrap();
        let size_avail =
            clean_mar_staging(&mar_fixture.mar_staging, max_total_size, SystemTime::now()).unwrap();
        assert_eq!(size_avail, max_total_size);
        assert!(!path.exists());
    }

    #[rstest]
    fn keeps_recent_mar_entry(mut mar_fixture: MarCollectorFixture, max_total_size: u64) {
        let now = SystemTime::now();
        let path = mar_fixture.create_logentry_with_size_and_age(1, now);
        let size_avail = clean_mar_staging(&mar_fixture.mar_staging, max_total_size, now).unwrap();
        assert!(size_avail < max_total_size);
        assert!(path.exists());
    }

    #[rstest]
    fn removes_too_large_mar_entry(mut mar_fixture: MarCollectorFixture, max_total_size: u64) {
        let now = SystemTime::now();
        // NOTE: the entire directory will be larger than max_total_size due to the manifest.json.
        let path = mar_fixture.create_logentry_with_size_and_age(max_total_size, now);
        let size_avail = clean_mar_staging(&mar_fixture.mar_staging, max_total_size, now).unwrap();
        assert_eq!(size_avail, max_total_size);
        assert!(!path.exists());
    }

    #[fixture]
    fn max_total_size() -> u64 {
        1024
    }

    #[fixture]
    fn mar_fixture() -> MarCollectorFixture {
        MarCollectorFixture::new()
    }
}
