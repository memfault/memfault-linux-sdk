//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! MAR Entry
//!
//! Create or Parse from disk MAR entries.
//!
//! A MAR entry is a folder with a unique name, a manifest and some optional attachments.
//!
use crate::{mar::manifest::CompressionAlgorithm, util::disk_size::DiskSize};
use eyre::{eyre, Context, Result};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::BufReader;
use std::iter::once;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use uuid::Uuid;

use crate::network::NetworkConfig;
use crate::util::fs::move_file;

use super::manifest::{CollectionTime, Manifest, Metadata};

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

/// A tool to build new MAR entries. Use one of the constructor functions and
/// call save() to write to disk. Any files attached to this MAR entry will
/// be moved when save is called.
pub struct MarEntryBuilder {
    collection_time: CollectionTime,
    metadata: Metadata,
    attachments: Vec<PathBuf>,
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
    pub fn iterate_from_container(mar_staging: &Path) -> Result<MarEntryIterator> {
        let entries = fs::read_dir(mar_staging)
            .wrap_err(eyre!(
                "Unable to open MAR staging area: {}",
                mar_staging.display()
            ))?
            .filter_map(std::io::Result::ok)
            .map(|d| d.path())
            // Keep only directories
            .filter(|p| p.is_dir());

        Ok(MarEntryIterator {
            directories: entries.collect(),
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
        let buf_reader =
            BufReader::new(File::open(manifest).wrap_err("Error reading manifest file")?);
        let manifest: Manifest =
            serde_json::from_reader(buf_reader).wrap_err("Error parsing manifest file")?;
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

impl MarEntryBuilder {
    pub fn new(metadata: Metadata, attachments: Vec<PathBuf>) -> Result<Self> {
        Ok(Self {
            collection_time: CollectionTime::now()?,
            metadata,
            attachments,
        })
    }

    pub fn new_log(
        file: PathBuf,
        cid: Uuid,
        next_cid: Uuid,
        compression: CompressionAlgorithm,
    ) -> Result<Self> {
        Self::new(
            Metadata::new_log(
                file.file_name()
                    .ok_or(eyre!("Logfile should be a file."))?
                    .to_str()
                    .ok_or(eyre!("Invalid log filename."))?
                    .to_owned(),
                cid,
                next_cid,
                compression,
            ),
            vec![file],
        )
    }

    /// Consume this builder, writes the manifest and moves the attachment to the
    /// MAR storage area and returns a MAR entry.
    pub fn save(self, mar_staging: &Path, network_config: &NetworkConfig) -> Result<MarEntry> {
        let mar_entry_uuid = Uuid::new_v4();

        // Create a directory for this entry
        let path = mar_staging.to_owned().join(mar_entry_uuid.to_string());
        fs::create_dir(&path)?;

        // Move attachments
        for filepath in self.attachments {
            // We already check that attachments are file in the constructor so we ignore
            // non-files here.
            if let Some(filename) = filepath.file_name() {
                let target = path.join(filename);

                move_file(&filepath, &target)?;
            }
        }

        // Prepare manifest
        let manifest = Manifest::new(network_config, self.collection_time, self.metadata);

        // Write the manifest to a temp file
        let manifest_path = path.join("manifest.tmp");
        let manifest_file = fs::File::create(&manifest_path)
            .wrap_err_with(|| format!("Error opening manifest {}", manifest_path.display()))?;
        serde_json::to_writer(manifest_file, &manifest)?;

        // Rename the manifest to signal that this folder is complete
        let manifest_json_path = manifest_path.with_extension("json");
        fs::rename(&manifest_path, &manifest_json_path).wrap_err_with(|| {
            format!(
                "Error renaming manifest {} to {}",
                manifest_path.display(),
                manifest_json_path.display()
            )
        })?;

        Ok(MarEntry {
            path,
            uuid: mar_entry_uuid,
            manifest,
        })
    }

    pub fn estimated_entry_size(&self) -> DiskSize {
        let attachments_size: u64 = self
            .attachments
            .iter()
            .filter_map(|p| p.metadata().ok())
            .map(|m| m.len())
            .sum();

        // Add a bit extra for the overhead of the manifest.json and directory inode:
        const OVERHEAD_SIZE_ESTIMATE: u64 = 4096;
        DiskSize {
            bytes: attachments_size + OVERHEAD_SIZE_ESTIMATE,
            inodes: attachments_size + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;
    use crate::mar::test_utils::MarCollectorFixture;
    use crate::test_utils::setup_logger;

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
