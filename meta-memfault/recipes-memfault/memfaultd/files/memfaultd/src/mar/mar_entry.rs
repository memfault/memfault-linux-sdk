//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! MAR Entry
//!
//! Represents a MAR entry on disk and provides a parsing utility for the MAR staging area.
//!
//! A MAR entry is a folder with a unique name, a manifest and some optional attachments.
//!
use std::fs::{read_dir, File};
use std::io::BufReader;
use std::iter::once;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{collections::VecDeque, time::SystemTime};

use eyre::{eyre, Context, Result};
use uuid::Uuid;

use super::manifest::Manifest;

/// A candidate folder for inclusion in a MAR zip file
pub struct MarEntry {
    /// Path to the directory on disk where the MAR entry is stored.
    pub path: PathBuf,
    pub uuid: Uuid,
    pub manifest: Manifest,
}

/// An iterator over a list of paths that may contain a MarEntry.
/// Each is lazily transformed into a MarEntry.
pub struct MarEntryIterator {
    directories: VecDeque<PathBuf>,
}

impl Iterator for MarEntryIterator {
    type Item = Result<MarEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.directories.pop_front().map(MarEntry::from_path)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.directories.len(), Some(self.directories.len()))
    }
}

impl MarEntry {
    /// Go through all files in our staging area and make a list of paths to valid
    /// MAR entries to include in the next MAR file.
    ///
    /// A valid MAR entry is a directory, named with a valid uuid. and it must
    /// contain a manifest.json file.  To avoid synchronization issue, writers of
    /// MAR entries should write to manifest.lock and rename manifest.json when
    /// they are done (atomic operation).
    pub fn iterate_from_container(
        mar_staging: &Path,
    ) -> Result<impl Iterator<Item = Result<MarEntry>>> {
        let mut entries: Vec<(PathBuf, Option<SystemTime>)> = read_dir(mar_staging)
            .wrap_err(eyre!(
                "Unable to open MAR staging area: {}",
                mar_staging.display()
            ))?
            .filter_map(std::io::Result::ok)
            // Keep only directories
            .filter(|d| d.path().is_dir())
            // Collect the creation time so we can sort them
            .map(|d| (d.path(), d.metadata().and_then(|m| m.created()).ok()))
            .collect();

        // Sort entries from oldest to newest
        entries.sort_by(|a, b| a.1.cmp(&b.1));

        Ok(MarEntryIterator {
            directories: entries.into_iter().map(|e| e.0).collect(),
        })
    }

    /// Creates a MarEntry instance from a directory containing a manifest.json file.
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let uuid = Uuid::from_str(
            path.file_name()
                .ok_or_else(|| eyre!("{} is not a directory", path.display()))?
                .to_str()
                .ok_or_else(|| eyre!("{} is not a valid directory name", path.display()))?,
        )?;
        let manifest = path.join("manifest.json");
        if !manifest.exists() {
            return Err(eyre!(
                "{} does not contain a manifest file.",
                path.display()
            ));
        }
        let buf_reader = BufReader::new(
            File::open(&manifest)
                .wrap_err_with(|| format!("Error reading manifest file {:?}", manifest))?,
        );
        let manifest: Manifest = serde_json::from_reader(buf_reader)
            .wrap_err_with(|| format!("Error parsing manifest file {:?}", manifest))?;
        Ok(Self {
            path,
            uuid,
            manifest,
        })
    }

    /// Returns an iterator over the filenames of the manifest.json and attachments of this MAR entry.
    pub fn filenames(&self) -> impl Iterator<Item = String> {
        once("manifest.json".to_owned()).chain(self.manifest.attachments())
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use crate::mar::test_utils::MarCollectorFixture;
    use crate::test_utils::setup_logger;

    use super::*;

    #[rstest]
    fn collecting_from_empty_folder(_setup_logger: (), mar_fixture: MarCollectorFixture) {
        assert_eq!(
            MarEntry::iterate_from_container(&mar_fixture.mar_staging)
                .unwrap()
                .count(),
            0
        )
    }

    #[rstest]
    fn collecting_from_folder_with_partial_entries(
        _setup_logger: (),
        mut mar_fixture: MarCollectorFixture,
    ) {
        mar_fixture.create_empty_entry();
        mar_fixture.create_logentry();

        assert_eq!(
            MarEntry::iterate_from_container(&mar_fixture.mar_staging)
                .unwrap()
                .filter(|e| e.is_ok())
                .count(),
            // Only one entry should be picked up. The other one is ignored.
            1
        )
    }

    #[fixture]
    fn mar_fixture() -> MarCollectorFixture {
        MarCollectorFixture::new()
    }
}
