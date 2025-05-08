//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};
use std::{
    fs::{create_dir, create_dir_all, set_permissions, File},
    io::{BufWriter, Write},
    os::unix::prelude::PermissionsExt,
    path::PathBuf,
};

use crate::{
    mar::{CompressionAlgorithm, DeviceAttribute},
    metrics::{MetricReportType, MetricStringKey, MetricValue},
    reboot::RebootReason,
};
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

    pub fn create_corrupted_manifest_entry(&mut self) -> PathBuf {
        let uuid = Uuid::new_v4();
        let path = self.mar_staging.join(uuid.to_string());
        create_dir(&path).unwrap();
        let manifest_path = path.join("manifest.json");

        let mut manifest_file = File::create(manifest_path).unwrap();
        manifest_file
            .write_all("{ \"foo\": 1.0 }".as_bytes())
            .unwrap();
        path
    }

    pub fn create_device_attributes_entry(
        &mut self,
        attributes: Vec<DeviceAttribute>,
        timestamp: SystemTime,
    ) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");

        let manifest_file = File::create(manifest_path).unwrap();

        let mut collection_time = CollectionTime::test_fixture();
        collection_time.timestamp = timestamp.into();

        let manifest = Manifest::new(
            &self.config,
            collection_time,
            Metadata::new_device_attributes(attributes),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

        path
    }

    pub fn create_logentry_with_size(&mut self, size: u64) -> PathBuf {
        self.create_logentry_with_size_and_age(size, SystemTime::now())
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

        let manifest_file = File::create(manifest_path).unwrap();

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

        let manifest_file = File::create(manifest_path).unwrap();
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
        File::create(manifest_path)
            .unwrap()
            .write_all(b"BOGUS")
            .unwrap();
        path
    }

    pub fn create_entry_without_directory_read_permission(&mut self) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");
        File::create(manifest_path)
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

    pub fn create_metric_report_entry(
        &mut self,
        metrics: HashMap<MetricStringKey, MetricValue>,
        duration: Duration,
        boottime_duration: Option<Duration>,
        report_type: MetricReportType,
    ) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");

        let manifest_file = File::create(manifest_path).unwrap();
        let manifest = Manifest::new(
            &self.config,
            CollectionTime::test_fixture(),
            Metadata::new_metric_report(metrics, duration, boottime_duration, report_type),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

        path
    }

    pub fn create_reboot_entry(&mut self, reason: RebootReason) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");

        let manifest_file = File::create(manifest_path).unwrap();
        let manifest = Manifest::new(
            &self.config,
            CollectionTime::test_fixture(),
            Metadata::new_reboot(reason),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

        path
    }

    pub fn create_custom_data_recording_entry(&mut self, data: Vec<u8>) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");
        let data_path = path.join("data");

        let mut data_file = File::create(&data_path).unwrap();
        data_file.write_all(&data).unwrap();

        let manifest_file = File::create(manifest_path).unwrap();
        let manifest = Manifest::new(
            &self.config,
            CollectionTime::test_fixture(),
            Metadata::new_custom_data_recording(
                None,
                Duration::from_secs(0),
                vec!["mime".to_string()],
                "test".to_string(),
                data_path.to_str().unwrap().to_string(),
                None,
            ),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

        path
    }

    pub fn create_device_config_entry(&mut self) -> PathBuf {
        let path = self.create_empty_entry();
        let manifest_path = path.join("manifest.json");

        let manifest_file = File::create(manifest_path).unwrap();

        let collection_time = CollectionTime::test_fixture();

        let manifest = Manifest::new(
            &self.config,
            collection_time,
            Metadata::new_device_config(42),
        );
        serde_json::to_writer(BufWriter::new(manifest_file), &manifest).unwrap();

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
