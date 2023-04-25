//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Coredump Uploader

use std::ffi::OsStr;
use std::{fs::remove_file, path::Path};

use eyre::Result;
use log::{error, info, warn};

use crate::retriable_error::IgnoreNonRetriableError;
use crate::util::fs::get_files_sorted_by_mtime;

/// A helper function for retrieving all coredumps and uploading them.
///
/// The provided function `uploader` should upload the given path, and
/// report Ok on success.
pub(crate) fn process_coredumps_with<F>(coredump_dir: &Path, mut uploader: F) -> Result<()>
where
    F: FnMut(&Path, bool) -> Result<()>,
{
    // No coredump directory means nothing to do.
    if !coredump_dir.exists() {
        return Ok(());
    }

    // Get all files in the coredump directory, sorted by mtime
    match get_files_sorted_by_mtime(coredump_dir) {
        Ok(coredumps) => {
            for path in coredumps {
                // ... attempt to upload it with the provided method
                let gzipped = matches!(path.extension().and_then(OsStr::to_str), Some("gz"));
                uploader(path.as_path(), gzipped)
                    // If it's a perma-error, log then pretend it was okay
                    .ignore_non_retriable_errors_with(|e| {
                        warn!("Error processing {:?}: {:#}", path, e);
                    })
                    // Otherwise, add specific context and return early
                    .map_err(|e| {
                        info!("Temporary error processing {:?}: {:#}", path, e);
                        e
                    })?;

                // Delete the coredump file, but DON'T halt uploading if this fails
                if let Err(e) = remove_file(&path) {
                    warn!("Failed to delete coredump file {:?}: {:#}", path, e);
                }
            }
        }
        Err(e) => {
            error!("Failed to read coredump directory: {:#}", e);
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use eyre::eyre;
    use mockall::automock;
    use mockall::predicate::{always, eq};
    use rstest::{fixture, rstest};
    use std::fs::{create_dir_all, read_dir, File};
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    use crate::retriable_error::RetriableError;

    use super::*;

    #[rstest]
    fn test_coredump_dir_non_existing(
        coredump_fixture: CoredumpFixture,
        mock_uploader: MockUploader,
    ) {
        let tmp_dir = coredump_fixture.tmp_dir.clone();
        drop(coredump_fixture); // Drop the fixture so the directory is deleted.

        // NB: mock_uploader.upload is never called, so we don't need to expect anything
        let result = process_coredumps_with(&tmp_dir, |path, gzipped| {
            mock_uploader.upload(path, gzipped)
        });
        assert!(result.is_ok());
    }

    #[rstest]
    #[case("coredump.gz", true)]
    #[case("coredump", false)]
    fn test_happy_path(
        mut coredump_fixture: CoredumpFixture,
        mut mock_uploader: MockUploader,
        #[case] filename: &str,
        #[case] gzipped: bool,
    ) {
        let coredump_path = coredump_fixture.create_coredump_file(filename);

        mock_uploader
            .expect_upload()
            .with(eq(coredump_path.clone()), eq(gzipped))
            .times(1)
            .returning(|_, _| Ok(()));

        let result = process_coredumps_with(&coredump_fixture.tmp_dir, |path, gzipped| {
            mock_uploader.upload(path, gzipped)
        });
        assert!(result.is_ok());
        // File is deleted after processing:
        assert!(!coredump_path.exists());
    }

    #[rstest]
    fn test_error_handling(mut coredump_fixture: CoredumpFixture, mut mock_uploader: MockUploader) {
        coredump_fixture.create_coredump_file("coredump1.gz");
        coredump_fixture.create_coredump_file("coredump2.gz");

        mock_uploader
            .expect_upload()
            .with(always(), always())
            .times(1)
            .returning(|_, _| Err(eyre!("Some error")));
        mock_uploader
            .expect_upload()
            .with(always(), always())
            .times(1)
            .returning(|_, _| Err(eyre!(RetriableError::ServerError { status_code: 503 })));

        let result = process_coredumps_with(&coredump_fixture.tmp_dir, |path, gzipped| {
            mock_uploader.upload(path, gzipped)
        });
        assert!(matches!(result, Err(e) if e.downcast_ref::<RetriableError>().is_some()));

        // One of the files will be uploaded and deleted, the other one is should still pending:
        assert_eq!(coredump_fixture.count_coredumps(), 1);
    }

    #[automock]
    trait Uploader {
        fn upload(&self, path: &Path, gzipped: bool) -> Result<()>;
    }

    #[fixture]
    fn mock_uploader() -> MockUploader {
        MockUploader::new()
    }

    #[fixture]
    fn coredump_fixture() -> CoredumpFixture {
        CoredumpFixture::new()
    }

    struct CoredumpFixture {
        #[allow(dead_code)]
        tmp_dir_handle: TempDir,
        tmp_dir: PathBuf,
    }

    impl CoredumpFixture {
        fn new() -> CoredumpFixture {
            let tmp_dir_handle = tempdir().unwrap();
            let tmp_dir = tmp_dir_handle.path().to_owned();
            create_dir_all(&tmp_dir).unwrap();
            CoredumpFixture {
                tmp_dir_handle,
                tmp_dir,
            }
        }
        fn create_coredump_file(&mut self, filename: &str) -> PathBuf {
            let path = self.tmp_dir.join(filename);
            File::create(&path).unwrap();
            path
        }
        fn count_coredumps(&self) -> usize {
            read_dir(&self.tmp_dir).unwrap().count()
        }
    }
}
