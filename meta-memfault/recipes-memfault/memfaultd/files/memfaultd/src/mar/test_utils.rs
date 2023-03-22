//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::time::SystemTime;
use std::{
    fs::{create_dir, create_dir_all, set_permissions, File},
    io::{BufWriter, Write},
    os::unix::prelude::PermissionsExt,
    path::PathBuf,
};

use crate::mar::manifest::CompressionAlgorithm;
use tempfile::{tempdir, TempDir};
use uuid::Uuid;

use crate::network::NetworkConfig;
use crate::test_utils::create_file_with_size;
use crate::util::zip::ZipEncoder;

use super::manifest::{CollectionTime, Manifest, Metadata};

pub struct MarCollectorFixture {
    pub mar_staging: PathBuf,
    // Keep a reference to the tempdir so it is automatically
    // deleted *after* the fixture
    _tempdir: TempDir,
    config: NetworkConfig,
}

impl MarCollectorFixture {
    pub fn new() -> Self {
        let tempdir = tempdir().unwrap();
        let mar_staging = tempdir.path().to_owned();
        create_dir_all(&mar_staging).unwrap();
        Self {
            mar_staging,
            _tempdir: tempdir,
            config: NetworkConfig::test_fixture(),
        }
    }

    pub fn create_empty_entry(&mut self) -> PathBuf {
        let uuid = Uuid::new_v4();
        let path = self.mar_staging.join(uuid.to_string());
        create_dir(&path).unwrap();
        path
    }

    pub fn create_logentry_with_size(&mut self, size: u64) -> PathBuf {
        return self.create_logentry_with_size_and_age(size, SystemTime::now());
    }

    pub fn create_logentry_with_size_and_age(
        &mut self,
        size: u64,
        timestamp: SystemTime,
    ) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");

        let log_name = "system.log".to_owned();
        let log_path = path.join(&log_name);
        create_file_with_size(&log_path, size).unwrap();

        let manifest_file = File::create(&manifest_path).unwrap();

        let mut collection_time = CollectionTime::test_fixture();
        collection_time.timestamp = timestamp.into();

        let manifest = Manifest::new(
            &self.config,
            collection_time,
            Metadata::new_log(
                log_name,
                Uuid::new_v4(),
                Uuid::new_v4(),
                CompressionAlgorithm::Zlib,
            ),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

        path
    }

    pub fn create_logentry(&mut self) -> PathBuf {
        self.create_logentry_with_size_and_age(0, SystemTime::now())
    }

    pub fn create_logentry_with_unreadable_attachment(&mut self) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");

        let log_name = "system.log".to_owned();
        let log_path = path.join(&log_name);
        let log = File::create(&log_path).unwrap();
        drop(log);

        let mut permissions = log_path.metadata().unwrap().permissions();
        permissions.set_mode(0o0);
        set_permissions(&log_path, permissions).unwrap();

        let manifest_file = File::create(&manifest_path).unwrap();
        let manifest = Manifest::new(
            &self.config,
            CollectionTime::test_fixture(),
            Metadata::new_log(
                log_name,
                Uuid::new_v4(),
                Uuid::new_v4(),
                CompressionAlgorithm::Zlib,
            ),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

        path
    }

    pub fn create_entry_with_bogus_json(&mut self) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");
        File::create(&manifest_path)
            .unwrap()
            .write_all(b"BOGUS")
            .unwrap();
        path
    }

    pub fn create_entry_without_directory_read_permission(&mut self) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");
        File::create(&manifest_path)
            .unwrap()
            .write_all(b"BOGUS")
            .unwrap();

        let mut permissions = path.metadata().unwrap().permissions();
        permissions.set_mode(0o0);
        set_permissions(&path, permissions).unwrap();
        path
    }

    pub fn create_entry_without_manifest_read_permission(&mut self) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");
        File::create(&manifest_path)
            .unwrap()
            .write_all(b"BOGUS")
            .unwrap();

        let mut permissions = manifest_path.metadata().unwrap().permissions();
        permissions.set_mode(0o0);
        set_permissions(manifest_path, permissions).unwrap();
        path
    }
}

/// Check the content of a MAR zip encoder against a list of expected files.
/// The first (in zip order) entry name is renamed from "some_uuid/" to
/// "<entry>/" before matching and the list is sorted alphabetically.
/// Eg: ZIP(abcd42/manifest.json abcd42/file.txt) => [<entry>/file.txt, <entry>/manifest.json]
pub fn assert_mar_content_matches(zip_encoder: &ZipEncoder, expected_files: Vec<&str>) -> bool {
    // Get the folder name for the first entry, we will s/(entry_uuid)/<entry>/ to make matching friendlier
    let file_names = zip_encoder.file_names();
    assert!(!file_names.is_empty());

    let entry_name = file_names[0]
        .split(std::path::MAIN_SEPARATOR)
        .next()
        .unwrap();
    let mut files_list = file_names
        .iter()
        .map(|filename| filename.replace(entry_name, "<entry>"))
        .collect::<Vec<String>>();
    files_list.sort();

    assert_eq!(files_list, *expected_files);
    true
}
