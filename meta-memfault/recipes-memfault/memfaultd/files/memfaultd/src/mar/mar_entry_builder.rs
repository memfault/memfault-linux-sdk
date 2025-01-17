//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! MAR Entry Builder
//!
use crate::network::NetworkConfig;
use crate::util::disk_size::DiskSize;
use crate::util::fs::move_file;
use crate::{
    mar::{CollectionTime, Manifest, MarEntry, Metadata},
    util::fs::copy_file,
};
use eyre::{eyre, Result, WrapErr};
use std::ffi::OsStr;
use std::fs::{create_dir, remove_dir_all, rename, File, Metadata as FsMetadata};
use std::io;
use std::mem::take;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE: u64 = 4096;

/// A tool to build new MAR entries. Use one of the constructor functions and
/// call save() to write to disk. Any files attached to this MAR entry will
/// be moved when save is called.
pub struct MarEntryBuilder<M> {
    entry_dir: MarEntryDir,
    uuid: Uuid,
    collection_time: CollectionTime,
    metadata: M,
    attachments: Vec<MarAttachment>,
}

pub struct NoMetadata;

impl<M> MarEntryBuilder<M> {
    fn entry_dir_path(&self) -> &Path {
        &self.entry_dir.path
    }

    pub fn make_attachment_path_in_entry_dir<F: AsRef<str>>(&self, filename: F) -> PathBuf {
        self.entry_dir_path().join(filename.as_ref())
    }

    /// Add an attachment to this entry.
    ///
    /// The file will be moved to the entry directory
    pub fn add_attachment(mut self, file: PathBuf) -> Result<MarEntryBuilder<M>> {
        if file.is_file() && file.is_absolute() {
            self.attachments.push(MarAttachment::Move(file));
            Ok(self)
        } else {
            Err(eyre!("Failed to add attachment!"))
        }
    }

    /// Add an attachment to this entry.
    ///
    /// The file will be copied to the entry directory
    pub fn add_copied_attachment(mut self, file: PathBuf) -> Result<MarEntryBuilder<M>> {
        if file.is_file() && file.is_absolute() {
            self.attachments.push(MarAttachment::Copy(file));
            Ok(self)
        } else {
            Err(eyre!("Failed to add copied attachment!"))
        }
    }
}

impl MarEntryBuilder<NoMetadata> {
    pub fn new(mar_staging: &Path) -> eyre::Result<MarEntryBuilder<NoMetadata>> {
        let collection_time = CollectionTime::now()?;

        // Create a directory for this entry. Make sure this is the last fallible operation here,
        // to avoid complicating cleanup in failure scenarios.
        let uuid = Uuid::new_v4();
        let path = mar_staging.to_owned().join(uuid.to_string());
        create_dir(&path)?;

        Ok(Self {
            entry_dir: MarEntryDir::new(path),
            uuid,
            collection_time,
            metadata: NoMetadata,
            attachments: vec![],
        })
    }

    pub fn set_metadata(self, metadata: Metadata) -> MarEntryBuilder<Metadata> {
        MarEntryBuilder {
            entry_dir: self.entry_dir,
            uuid: self.uuid,
            collection_time: self.collection_time,
            attachments: self.attachments,
            metadata,
        }
    }
}

impl MarEntryBuilder<Metadata> {
    /// Consume this builder, writes the manifest and moves the attachment to the
    /// MAR storage area and returns a MAR entry.
    pub fn save(self, network_config: &NetworkConfig) -> Result<MarEntry> {
        // Move attachments
        for filepath in self.attachments {
            // We already check that attachments are file in the constructor so we ignore
            // non-files here.
            if let Some(filename) = filepath.file_name() {
                let target = self.entry_dir.path.join(filename);

                match filepath {
                    MarAttachment::Copy(source) => {
                        copy_file(&source, &target)?;
                    }
                    MarAttachment::Move(source) => {
                        move_file(&source, &target)?;
                    }
                }
            }
        }

        // Prepare manifest
        let manifest = Manifest::new(network_config, self.collection_time, self.metadata);

        // Write the manifest to a temp file
        let manifest_path = self.entry_dir.path.join("manifest.tmp");
        let manifest_file = File::create(&manifest_path)
            .wrap_err_with(|| format!("Error opening manifest {}", manifest_path.display()))?;
        serde_json::to_writer(manifest_file, &manifest)?;

        // Rename the manifest to signal that this folder is complete
        let manifest_json_path = manifest_path.with_extension("json");
        rename(&manifest_path, &manifest_json_path).wrap_err_with(|| {
            format!(
                "Error renaming manifest {} to {}",
                manifest_path.display(),
                manifest_json_path.display()
            )
        })?;

        Ok(MarEntry {
            path: self.entry_dir.mark_saved(),
            uuid: self.uuid,
            manifest,
        })
    }

    pub fn estimated_entry_size(&self) -> DiskSize {
        let attachments_size_bytes: u64 = self
            .attachments
            .iter()
            .filter_map(|p| p.metadata().ok())
            .map(|m| m.len())
            .sum();

        // Add a bit extra for the overhead of the manifest.json and directory inode:
        DiskSize {
            bytes: attachments_size_bytes + MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE,
            inodes: self.attachments.len() as u64 + 1,
        }
    }

    pub fn get_metadata(&self) -> &Metadata {
        &self.metadata
    }
}

#[derive(Debug)]
enum MarAttachment {
    Copy(PathBuf),
    Move(PathBuf),
}

impl MarAttachment {
    fn file_name(&self) -> Option<&OsStr> {
        match self {
            MarAttachment::Copy(path) | MarAttachment::Move(path) => path.file_name(),
        }
    }

    fn metadata(&self) -> io::Result<FsMetadata> {
        match self {
            MarAttachment::Copy(path) | MarAttachment::Move(path) => path.metadata(),
        }
    }
}

/// Helper structure that will clean up the entry directory on Drop if mark_saved() was not called.
struct MarEntryDir {
    path: PathBuf,
    saved: bool,
}

impl MarEntryDir {
    fn new(path: PathBuf) -> Self {
        Self { path, saved: false }
    }

    fn mark_saved(mut self) -> PathBuf {
        self.saved = true;
        take(&mut self.path)
    }
}

impl Drop for MarEntryDir {
    fn drop(&mut self) {
        if !self.saved {
            let _ = remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE;
    use crate::mar::MarEntryBuilder;
    use crate::mar::Metadata;
    use crate::network::NetworkConfig;
    use crate::test_utils::create_file_with_size;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    #[rstest]
    fn cleans_up_entry_dir_when_save_was_not_called(fixture: Fixture) {
        let builder = MarEntryBuilder::new(&fixture.mar_staging).unwrap();
        let entry_dir = builder.entry_dir_path().to_owned();
        assert!(entry_dir.exists());
        create_file_with_size(&entry_dir.join("attachment"), 1024).unwrap();
        drop(builder);
        assert!(!entry_dir.exists());
    }

    #[rstest]
    fn save_keeps_entry_dir_and_adds_manifest_json(fixture: Fixture) {
        let mut entry_dir_option = None;
        {
            let builder = MarEntryBuilder::new(&fixture.mar_staging).unwrap();
            let _ = entry_dir_option.insert(builder.entry_dir_path().to_owned());
            builder
                .set_metadata(Metadata::test_fixture())
                .save(&NetworkConfig::test_fixture())
                .unwrap();
        }
        let entry_dir = entry_dir_option.unwrap();
        assert!(entry_dir.exists());
        assert!(entry_dir.join("manifest.json").exists());
    }

    #[rstest]
    fn create_attachment_inside_entry_dir(fixture: Fixture) {
        let builder = MarEntryBuilder::new(&fixture.mar_staging).unwrap();
        let orig_attachment_path = builder.make_attachment_path_in_entry_dir("attachment");
        create_file_with_size(&orig_attachment_path, 1024).unwrap();

        builder
            .add_attachment(orig_attachment_path.clone())
            .unwrap()
            .set_metadata(Metadata::test_fixture())
            .save(&NetworkConfig::test_fixture())
            .unwrap();

        // Attachment is still where it was written:
        assert!(orig_attachment_path.exists());
    }

    #[rstest]
    fn attachment_outside_entry_dir_is_moved_into_entry_dir_upon_save(fixture: Fixture) {
        let builder = MarEntryBuilder::new(&fixture.mar_staging).unwrap();
        let entry_dir = builder.entry_dir_path().to_owned();

        let tempdir = tempdir().unwrap();
        let orig_attachment_path = tempdir.path().join("attachment");
        create_file_with_size(&orig_attachment_path, 1024).unwrap();

        builder
            .add_attachment(orig_attachment_path.clone())
            .unwrap()
            .set_metadata(Metadata::test_fixture())
            .save(&NetworkConfig::test_fixture())
            .unwrap();

        // Attachment has been moved into the entry dir:
        assert!(!orig_attachment_path.exists());
        assert!(entry_dir
            .join(orig_attachment_path.file_name().unwrap())
            .exists());
    }

    #[rstest]
    fn can_estimate_size_of_a_mar_entry(fixture: Fixture) {
        let builder = MarEntryBuilder::new(&fixture.mar_staging).unwrap();
        let orig_attachment_path = builder.make_attachment_path_in_entry_dir("attachment");
        create_file_with_size(&orig_attachment_path, 1024).unwrap();

        let builder = builder
            .add_attachment(orig_attachment_path)
            .unwrap()
            .set_metadata(Metadata::test_fixture());

        assert_eq!(
            builder.estimated_entry_size().bytes,
            1024 + MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE
        );
        assert_eq!(builder.estimated_entry_size().inodes, 2);
    }

    struct Fixture {
        _tempdir: TempDir,
        mar_staging: PathBuf,
    }

    #[fixture]
    fn fixture() -> Fixture {
        let tempdir = tempdir().unwrap();
        let mar_staging = tempdir.path().to_owned();
        Fixture {
            _tempdir: tempdir,
            mar_staging,
        }
    }
}
