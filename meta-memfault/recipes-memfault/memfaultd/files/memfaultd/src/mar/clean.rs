//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{
    mar::MarEntry,
    metrics::{
        internal_metrics::{
            INTERNAL_METRIC_MAR_CLEANER_DURATION, INTERNAL_METRIC_MAR_ENTRIES_DELETED,
            INTERNAL_METRIC_MAR_ENTRY_COUNT,
        },
        KeyedMetricReading, MetricStringKey, MetricsMBox,
    },
    util::disk_size::{get_disk_space, get_size, DiskSize},
};
use eyre::{eyre, Result, WrapErr};
use log::{debug, trace, warn};
use std::fs::read_dir;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{
    fmt::{Display, Formatter, Result as FmtResult},
    time::Instant,
};

pub struct MarStagingCleaner {
    mar_staging_path: PathBuf,
    max_total_size: DiskSize,
    min_headroom: DiskSize,
    max_age: Duration,
    max_count: usize,
    metrics_mbox: MetricsMBox,
}

impl MarStagingCleaner {
    pub fn new(
        mar_staging_path: &Path,
        max_total_size: DiskSize,
        min_headroom: DiskSize,
        max_age: Duration,
        max_count: usize,
        metrics_mbox: MetricsMBox,
    ) -> Self {
        Self {
            mar_staging_path: mar_staging_path.to_owned(),
            max_total_size,
            min_headroom,
            max_age,
            max_count,
            metrics_mbox,
        }
    }

    /// Cleans up MAR entries in the staging folder.
    /// Returns the amount of space left until the max_total_size will be reached or until
    /// min_headroom will be exceeded (the smallest of the two values).
    pub fn clean(&self, required_space: DiskSize) -> Result<DiskSize> {
        trace!("Cleaning MAR staging area...");
        clean_mar_staging(
            &self.mar_staging_path,
            self.max_total_size.saturating_sub(required_space),
            get_disk_space(&self.mar_staging_path).unwrap_or(DiskSize::ZERO),
            self.min_headroom + required_space,
            SystemTime::now(),
            self.max_age,
            self.max_count,
            &self.metrics_mbox,
        )
        .map_err(|e| {
            warn!("Unable to clean MAR entries: {}", e);
            e
        })
    }
}

#[derive(Debug)]
enum DeletionReason {
    Expired,
    DiskQuota,
    EntryQuota,
}
impl Display for DeletionReason {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match self {
            DeletionReason::Expired => write!(f, "Expired"),
            DeletionReason::DiskQuota => write!(f, "Disk quota"),
            DeletionReason::EntryQuota => write!(f, "Entry quota"),
        }
    }
}

#[derive(Debug, Clone)]
struct AgeSizePath {
    age: Duration,
    size: DiskSize,
    path: PathBuf,
}

impl AgeSizePath {
    fn new(
        path: &Path,
        size: DiskSize,
        timestamp: SystemTime,
        reference_date: SystemTime,
    ) -> AgeSizePath {
        Self {
            age: (reference_date.duration_since(timestamp)).unwrap_or(Duration::ZERO),
            size,
            path: path.to_owned(),
        }
    }
}

// TODO: Refactor clean_mar_staging so
// that it complies with this lint
#[allow(clippy::too_many_arguments)]
fn clean_mar_staging(
    mar_staging: &Path,
    max_total_size: DiskSize,
    available_space: DiskSize,
    min_space: DiskSize,
    reference_date: SystemTime,
    max_age: Duration,
    max_count: usize,
    metrics_mbox: &MetricsMBox,
) -> Result<DiskSize> {
    let cleaning_start = Instant::now();
    let (entries, total_space_used) = collect_mar_entries(mar_staging, reference_date)?;
    let total_entries = entries.len() as u64;

    let marked_entries = mark_entries_for_deletion(
        entries,
        total_space_used,
        max_total_size,
        available_space,
        min_space,
        max_age,
        max_count,
    );

    let (space_freed, entries_deleted) = remove_marked_entries(marked_entries);

    let elapsed_clean_time = cleaning_start.elapsed().as_secs_f64();

    let remaining_quota =
        max_total_size.saturating_sub(total_space_used.saturating_sub(space_freed));
    let usable_space = (available_space + space_freed).saturating_sub(min_space);

    let entries_deleted_counter = KeyedMetricReading::new_counter(
        MetricStringKey::from(INTERNAL_METRIC_MAR_ENTRIES_DELETED),
        entries_deleted as f64,
    );

    let mar_entry_count = KeyedMetricReading::new_histogram(
        MetricStringKey::from(INTERNAL_METRIC_MAR_ENTRY_COUNT),
        total_entries as f64,
    );

    let cleaning_duration = KeyedMetricReading::new_histogram(
        MetricStringKey::from(INTERNAL_METRIC_MAR_CLEANER_DURATION),
        elapsed_clean_time,
    );

    if metrics_mbox
        .send_and_forget([entries_deleted_counter, cleaning_duration, mar_entry_count].into())
        .is_err()
    {
        debug!("Couldn't send MAR entries deleted counter reading")
    }

    // Available space to write is the min of bytes and inodes remaining.
    Ok(DiskSize::min(remaining_quota, usable_space))
}

fn collect_mar_entries(
    mar_staging: &Path,
    reference_date: SystemTime,
) -> Result<(Vec<AgeSizePath>, DiskSize)> {
    let entries: Vec<AgeSizePath> = read_dir(mar_staging)
        .wrap_err(eyre!(
            "Unable to open MAR staging area: {}",
            mar_staging.display()
        ))?
        .filter_map(|r| r.map_err(|e| warn!("Unable to read DirEntry: {}", e)).ok())
        .map(|dir_entry| match MarEntry::from_path(dir_entry.path()) {
            // Use the collection time from the manifest or folder creation time if the manifest cannot be parsed:
            Ok(entry) => AgeSizePath::new(
                &entry.path,
                get_size(&entry.path).unwrap_or(DiskSize::ZERO),
                entry.manifest.collection_time.timestamp.into(),
                reference_date,
            ),
            Err(_) => {
                let path = dir_entry.path();
                let timestamp = path
                    .metadata()
                    .and_then(|m| m.created())
                    .unwrap_or_else(|_| SystemTime::now());
                AgeSizePath::new(
                    &path,
                    get_size(&path).unwrap_or(DiskSize::ZERO),
                    timestamp,
                    reference_date,
                )
            }
        })
        .collect();
    let total_space_used = entries
        .iter()
        .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);
    Ok((entries, total_space_used))
}

fn mark_entries_for_deletion(
    entries: Vec<AgeSizePath>,
    total_space_used: DiskSize,
    max_total_size: DiskSize,
    available_space: DiskSize,
    min_space: DiskSize,
    max_age: Duration,
    max_count: usize,
) -> Vec<(AgeSizePath, DeletionReason)> {
    // Sort entries with oldest first
    let mut entries_by_age = entries;
    entries_by_age.sort_by(|a, b| b.age.cmp(&a.age));

    // Note that (total_space_used).exceeds(max_total_size) would not
    // work here because it would be false when only one of bytes/inode is
    // exceeded. (we have a test to verify this)
    let max_total_size_exceeded = !max_total_size.exceeds(&(total_space_used));
    // Same here, min_space.exceeds(available_space) is sufficient but not necessary.
    let min_headroom_exceeded = !available_space.exceeds(&min_space);

    // Calculate how much space needs to be freed based on quotas
    let need_to_free = match (max_total_size_exceeded, min_headroom_exceeded) {
        (false, false) => DiskSize::ZERO,
        (true, false) => total_space_used.saturating_sub(max_total_size),
        (false, true) => min_space.saturating_sub(available_space),
        // If we have exceeded both quotas, need to free the larger difference
        // for both inodes and bytes
        (true, true) => DiskSize::max(
            total_space_used.saturating_sub(max_total_size),
            min_space.saturating_sub(available_space),
        ),
    };

    let mut space_to_be_freed = DiskSize::ZERO;

    // Ignore max_age if it is configured to 0
    let delete_expired_entries = !max_age.is_zero();

    // Ignore max_count if it is configured to 0
    let mut over_max_entries = match max_count {
        0 => 0,
        _ => entries_by_age.len().saturating_sub(max_count),
    };

    // Since the vector is sorted from oldest to newest,
    // older entries will be marked for deletion first
    entries_by_age
        .into_iter()
        .filter_map(|entry| {
            if over_max_entries > 0 {
                over_max_entries -= 1;
                space_to_be_freed += entry.size;
                Some((entry, DeletionReason::EntryQuota))
            } else if need_to_free != DiskSize::ZERO && !space_to_be_freed.exceeds(&need_to_free) {
                space_to_be_freed += entry.size;
                Some((entry, DeletionReason::DiskQuota))
            } else if delete_expired_entries && entry.age > max_age {
                space_to_be_freed += entry.size;
                Some((entry, DeletionReason::Expired))
            } else {
                None
            }
        })
        .collect()
}

fn remove_marked_entries(marked_entries: Vec<(AgeSizePath, DeletionReason)>) -> (DiskSize, u64) {
    let mut space_freed = DiskSize::ZERO;
    let mut entries_freed = 0;
    for (entry, deletion_reason) in marked_entries {
        debug!(
            "Cleaning up MAR entry: {} ({} bytes / {} inodes, ~{} seconds old). Deletion reason: {}",
            entry.path.display(),
            entry.size.bytes,
            entry.size.inodes,
            entry.age.as_secs(),
            deletion_reason
        );
        if let Err(e) = std::fs::remove_dir_all(&entry.path) {
            warn!("Unable to remove MAR entry: {}", e);
        } else {
            debug!(
                "Removed MAR entry: {} {:?}",
                entry.path.display(),
                entry.size
            );
            space_freed += entry.size;
            entries_freed += 1;
        }
    }
    (space_freed, entries_freed)
}

#[cfg(test)]
mod test {
    use crate::mar::test_utils::MarCollectorFixture;
    use crate::metrics::TakeMetrics;
    use crate::test_utils::create_file_with_size;
    use crate::test_utils::setup_logger;
    use insta::assert_json_snapshot;
    use rstest::{fixture, rstest};
    use ssf::ServiceMock;

    use super::*;
    #[rstest]
    fn test_collect_entries(mut mar_fixture: MarCollectorFixture) {
        let now = SystemTime::now();

        let path_a = mar_fixture.create_logentry_with_size_and_age(100, now);
        let path_b =
            mar_fixture.create_logentry_with_size_and_age(100, now + Duration::from_secs(5));
        let path_c =
            mar_fixture.create_logentry_with_size_and_age(100, now + Duration::from_secs(10));
        let (entries, _total_space_used) =
            collect_mar_entries(&mar_fixture.mar_staging, SystemTime::now()).unwrap();
        assert!(entries.len() == 3);
        assert!([path_a, path_b, path_c].iter().all(|path| entries
            .iter()
            .any(|entry| entry.path.to_str() == path.to_str())));
    }

    #[rstest]
    fn test_no_delete_reasons_when_within_quota() {
        let now = SystemTime::now();

        let max_total_size = DiskSize::new_capacity(25000);
        let min_headroom = DiskSize {
            bytes: 20000,
            inodes: 20,
        };
        let available_space = DiskSize {
            bytes: min_headroom.bytes,
            inodes: 100,
        };

        let entries = vec![
            AgeSizePath::new(
                Path::new("/mock/a"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now + Duration::from_secs(1),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/b"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now + Duration::from_secs(10),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/c"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now + Duration::from_secs(15),
                now,
            ),
        ];

        let total_space_used = entries
            .iter()
            .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);

        let marked_entries = mark_entries_for_deletion(
            entries,
            total_space_used,
            max_total_size,
            available_space,
            min_headroom,
            Duration::from_secs(0), // No max age for this test
            0,
        );

        let do_not_deletes = ["/mock/a", "/mock/b"];

        assert!(marked_entries.iter().all(|(entry, _reason)| !do_not_deletes
            .iter()
            .any(|&do_not_delete_path| entry.path.to_str().unwrap() == do_not_delete_path)));
    }

    #[rstest]
    #[case(DiskSize::new_capacity(2500))] // 1 entry over quota by bytes
    #[case(DiskSize {bytes: 10000, inodes: 4})] // 1 entry over quota by inodes
    fn test_oldest_marked_when_over_max_total_size(#[case] max_total_size: DiskSize) {
        let now = SystemTime::now();
        let entries = vec![
            AgeSizePath::new(
                Path::new("/mock/c"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(15),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/a"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(1),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/b"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(10),
                now,
            ),
        ];

        let total_space_used = entries
            .iter()
            .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);
        let marked_entries = mark_entries_for_deletion(
            entries,
            total_space_used,
            max_total_size,
            DiskSize::new_capacity(10000),
            DiskSize::ZERO,         // No min headroom quota for this test
            Duration::from_secs(0), // No max age for this test
            0,
        );

        let do_not_deletes = ["/mock/a", "/mock/b"];

        assert!(marked_entries.iter().all(|(entry, _reason)| !do_not_deletes
            .iter()
            .any(|&do_not_delete_path| entry.path.to_str().unwrap() == do_not_delete_path)));

        for (entry, reason) in marked_entries {
            if entry.path.to_str().unwrap() == "/mock/c" {
                assert!(matches!(reason, DeletionReason::DiskQuota));
            }
        }
    }

    #[rstest]
    #[case(DiskSize {bytes: 3000, inodes: 100}, DiskSize::new_capacity(1500))] // 2 entries over quota by bytes
    #[case(DiskSize {bytes: 10000, inodes: 4}, DiskSize {bytes: 10000, inodes: 0})] // 2 entries over quota by inodes
    fn test_two_oldest_marked_when_under_min_headroom(
        #[case] min_headroom: DiskSize,
        #[case] available_space: DiskSize,
    ) {
        let now = SystemTime::now();
        let entries = vec![
            AgeSizePath::new(
                Path::new("/mock/c"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(15),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/a"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now,
                now - Duration::from_secs(1),
            ),
            AgeSizePath::new(
                Path::new("/mock/b"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(10),
                now,
            ),
        ];

        let total_space_used = entries
            .iter()
            .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);

        let marked_entries = mark_entries_for_deletion(
            entries,
            total_space_used,
            DiskSize::new_capacity(10000),
            available_space,
            min_headroom,
            Duration::from_secs(0), // No max age for this test
            0,
        );

        assert!(marked_entries
            .iter()
            .all(|(entry, _reason)| entry.path.to_str().unwrap() != "/mock/a"));

        for (entry, reason) in marked_entries {
            match entry.path.to_str().unwrap() {
                "/mock/c" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                "/mock/b" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                _ => unreachable!(),
            }
        }
    }

    #[rstest]
    fn expired_entries_marked() {
        let now = SystemTime::now();
        let entries = vec![
            AgeSizePath::new(
                Path::new("/mock/c"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(15),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/a"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(5),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/b"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(10),
                now,
            ),
        ];

        let total_space_used = entries
            .iter()
            .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);
        let marked_entries = mark_entries_for_deletion(
            entries,
            total_space_used,
            DiskSize::new_capacity(10000),
            DiskSize::new_capacity(10000),
            DiskSize::ZERO,         // No min headroom quota for this test
            Duration::from_secs(1), // low max age so all entries are marked as expired
            0,
        );

        assert!(marked_entries
            .iter()
            .all(|(_entry, reason)| matches!(reason, DeletionReason::Expired)));
    }

    #[rstest]
    // 2 oldest entries need to be deleted to get under quota
    // 3rd oldest needs to be deleted because it is expired
    // 2 most recent should not be deleted
    fn test_marks_quota_and_expired_entries() {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(3000);

        let entries = vec![
            AgeSizePath::new(
                Path::new("/mock/c"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(200),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/d"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(250),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/e"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(300),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/a"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(15),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/b"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(30),
                now,
            ),
        ];

        let total_space_used = entries
            .iter()
            .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);
        let marked_entries = mark_entries_for_deletion(
            entries,
            total_space_used,
            max_total_size,
            DiskSize::new_capacity(10000),
            DiskSize::ZERO,
            Duration::from_secs(100),
            0,
        );

        let do_not_deletes = ["/mock/a", "/mock/b"];

        assert!(marked_entries.iter().all(|(entry, _reason)| !do_not_deletes
            .iter()
            .any(|&do_not_delete_path| entry.path.to_str().unwrap() == do_not_delete_path)));

        for (entry, reason) in marked_entries {
            match entry.path.to_str().unwrap() {
                "/mock/c" => assert!(matches!(reason, DeletionReason::Expired)),
                "/mock/d" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                "/mock/e" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                _ => unreachable!(),
            }
        }
    }

    #[rstest]
    // Over the max total space used quota by 2000 bytes
    // and under the minimum headroom quota by 6 inodes
    // Need to delete 3 entries altogether
    fn test_marks_when_different_quotas_exceeded() {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(3000);
        let min_headroom = DiskSize {
            bytes: 1024,
            inodes: 10,
        };
        let available_space = DiskSize {
            bytes: 10000,
            inodes: 6,
        };

        // 2 oldest entries need to be deleted to get under quota
        // 3rd oldest needs to be deleted because it is expired
        // 2 most recent should not be deleted
        let entries = vec![
            AgeSizePath::new(
                Path::new("/mock/c"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(200),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/d"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(250),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/e"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(300),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/a"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(15),
                now,
            ),
            AgeSizePath::new(
                Path::new("/mock/b"),
                DiskSize {
                    bytes: 1000,
                    inodes: 2,
                },
                now - Duration::from_secs(30),
                now,
            ),
        ];

        let total_space_used = entries
            .iter()
            .fold(DiskSize::ZERO, |space_used, entry| entry.size + space_used);
        let marked_entries = mark_entries_for_deletion(
            entries,
            total_space_used,
            max_total_size,
            available_space,
            min_headroom,
            Duration::from_secs(0), // No max age in this test
            0,
        );

        let do_not_deletes = ["/mock/a", "/mock/b"];

        assert!(marked_entries.iter().all(|(entry, _reason)| !do_not_deletes
            .iter()
            .any(|&do_not_delete_path| entry.path.to_str().unwrap() == do_not_delete_path)));
        for (entry, reason) in marked_entries {
            match entry.path.to_str().unwrap() {
                "/mock/c" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                "/mock/d" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                "/mock/e" => assert!(matches!(reason, DeletionReason::DiskQuota)),
                _ => unreachable!(),
            }
        }
    }

    #[rstest]
    fn empty_staging_area(
        mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            SystemTime::now(),
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space.saturating_sub(min_headroom))
        );
    }

    #[rstest]
    fn keeps_recent_unfinished_mar_entry(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let path = mar_fixture.create_empty_entry();
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            SystemTime::now(),
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space.saturating_sub(min_headroom))
        );
        assert!(path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_unfinished_mar_entry_exceeding_max_total_size(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let path = mar_fixture.create_empty_entry();

        create_file_with_size(&path.join("log.txt"), max_total_size.bytes + 1).unwrap();
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space.saturating_sub(DiskSize {
                bytes: 0,
                inodes: 1,
            }),
            min_headroom,
            SystemTime::now(),
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space.saturating_sub(min_headroom))
        );
        assert!(!path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn keeps_recent_mar_entry(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let now = SystemTime::now();
        let path = mar_fixture.create_logentry_with_size_and_age(1, now);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(max_total_size.exceeds(&size_avail));
        assert!(path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_mar_entry_exceeding_max_total_size(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let now = SystemTime::now();
        // NOTE: the entire directory will be larger than max_total_size due to the manifest.json.
        let path = mar_fixture.create_logentry_with_size_and_age(max_total_size.bytes, now);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space.saturating_sub(DiskSize {
                bytes: 0,
                inodes: 2,
            }),
            min_headroom,
            now,
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert_eq!(
            size_avail,
            DiskSize::min(max_total_size, available_space.saturating_sub(min_headroom))
        );
        assert!(!path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_mar_entry_exceeding_min_headroom(
        mut mar_fixture: MarCollectorFixture,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
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
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(!path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_oldest_mar_entry_exceeding_max_total_size_when_multiple(
        mut mar_fixture: MarCollectorFixture,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(23000);
        let min_headroom = DiskSize {
            bytes: 1024,
            inodes: 10,
        };
        let available_space = DiskSize {
            bytes: max_total_size.bytes * 2,
            inodes: 100,
        };
        let oldest =
            mar_fixture.create_logentry_with_size_and_age(8000, now - Duration::from_secs(120));
        let middle =
            mar_fixture.create_logentry_with_size_and_age(8000, now - Duration::from_secs(30));
        let most_recent = mar_fixture.create_logentry_with_size_and_age(8000, now);
        let _size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(!oldest.exists());
        assert!(middle.exists());
        assert!(most_recent.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_entries_exceeding_min_headroom_size_by_age(
        mut mar_fixture: MarCollectorFixture,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
        _setup_logger: (),
    ) {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(80000);
        let min_headroom = DiskSize {
            bytes: 20000,
            inodes: 20,
        };
        let available_space = DiskSize {
            bytes: min_headroom.bytes - 20000,
            inodes: 100,
        };
        let oldest =
            mar_fixture.create_logentry_with_size_and_age(10000, now - Duration::from_secs(120));
        let second_oldest =
            mar_fixture.create_logentry_with_size_and_age(10000, now - Duration::from_secs(30));
        let second_newest =
            mar_fixture.create_logentry_with_size_and_age(10000, now - Duration::from_secs(10));
        let most_recent = mar_fixture.create_logentry_with_size_and_age(10000, now);

        // Need to delete 2 entries to free up required headroom
        let _size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(most_recent.exists());
        assert!(second_newest.exists());
        assert!(!second_oldest.exists());
        assert!(!oldest.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_entries_exceeding_max_total_size_by_age(
        mut mar_fixture: MarCollectorFixture,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
        _setup_logger: (),
    ) {
        let now = SystemTime::now();
        let max_total_size = DiskSize::new_capacity(25000);
        let min_headroom = DiskSize {
            bytes: 1024,
            inodes: 20,
        };
        let available_space = DiskSize {
            bytes: max_total_size.bytes * 2,
            inodes: 100,
        };
        let oldest =
            mar_fixture.create_logentry_with_size_and_age(10000, now - Duration::from_secs(120));
        let second_oldest =
            mar_fixture.create_logentry_with_size_and_age(10000, now - Duration::from_secs(30));
        let second_newest =
            mar_fixture.create_logentry_with_size_and_age(10000, now - Duration::from_secs(10));
        let most_recent = mar_fixture.create_logentry_with_size_and_age(10000, now);
        let _size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(second_newest.exists());
        assert!(most_recent.exists());
        assert!(!oldest.exists());
        assert!(!second_oldest.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_mar_entry_exceeding_min_headroom_inodes(
        _setup_logger: (),
        mut mar_fixture: MarCollectorFixture,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
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
            Duration::from_secs(604800),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(!path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn removes_mar_entry_exceeding_max_age(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let now = SystemTime::now();
        let thirty_seconds_ago = now - Duration::from_secs(30);
        let path_unexpired = mar_fixture.create_logentry_with_size_and_age(1, thirty_seconds_ago);
        let ten_min_ago = now - Duration::from_secs(600);
        let path_expired = mar_fixture.create_logentry_with_size_and_age(1, ten_min_ago);

        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(60),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(path_unexpired.exists());
        assert!(!path_expired.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn keeps_mar_entry_within_max_age(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let now = SystemTime::now();
        let thirty_seconds_ago = now - Duration::from_secs(30);
        let path = mar_fixture.create_logentry_with_size_and_age(1, thirty_seconds_ago);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(60),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    fn keeps_mar_entry_when_max_age_is_zero(
        mut mar_fixture: MarCollectorFixture,
        max_total_size: DiskSize,
        available_space: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
        min_headroom: DiskSize,
    ) {
        let now = SystemTime::now();
        let over_one_week_ago = now - Duration::from_secs(604801);
        let path = mar_fixture.create_logentry_with_size_and_age(1, over_one_week_ago);
        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            now,
            Duration::from_secs(0),
            0,
            &metrics_service.mbox,
        )
        .unwrap();
        assert!(size_avail.bytes >= 1);
        assert!(path.exists());
        assert_json_snapshot!(metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
    }

    #[rstest]
    #[case::max_entries_0(0, (true, true), "max_entries_0")]
    #[case::max_entries_less_than(1, (false, true), "max_entries_less_than")]
    #[case::max_entries_equal(2, (true, true), "max_entries_equal")]
    #[case::max_entries_greater_than(3, (true, true), "max_entries_greater_than")]
    fn deletes_mar_entry_when_over_max_count(
        #[case] entry_quota: usize,
        #[case] keep_entries: (bool, bool),
        #[case] test_name: &str,
        mut mar_fixture: MarCollectorFixture,
        available_space: DiskSize,
        min_headroom: DiskSize,
        mut metrics_service: ServiceMock<Vec<KeyedMetricReading>>,
    ) {
        let (keep_old_entry, keep_new_entry) = keep_entries;
        let now = SystemTime::now();
        let yesterday = now - Duration::from_secs(86400);
        let old_dir = mar_fixture.create_logentry_with_size_and_age(1, yesterday);
        let now_dir = mar_fixture.create_logentry_with_size_and_age(1, now);

        let max_total_size = DiskSize::new_capacity(2048);

        let size_avail = clean_mar_staging(
            &mar_fixture.mar_staging,
            max_total_size,
            available_space,
            min_headroom,
            SystemTime::now(),
            Duration::from_secs(0),
            entry_quota,
            &metrics_service.mbox,
        );

        assert!(size_avail.is_ok());
        assert_eq!(old_dir.exists(), keep_old_entry);
        assert_eq!(now_dir.exists(), keep_new_entry);
        assert_json_snapshot!(test_name, metrics_service.take_metrics().unwrap(), {
            ".MemfaultSdkMetric_mar_clean_duration_seconds" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_max" => "<duration>",
            ".MemfaultSdkMetric_mar_clean_duration_seconds_min" => "<duration>"
        });
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

    #[fixture]
    fn metrics_service() -> ServiceMock<Vec<KeyedMetricReading>> {
        ServiceMock::new()
    }
}
