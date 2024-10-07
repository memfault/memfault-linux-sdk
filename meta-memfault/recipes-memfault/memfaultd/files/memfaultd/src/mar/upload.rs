//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect and upload MAR entries.
//!
//! This module provides the functionality to collect all valid MAR entries, upload them and delete them on success.
//!
//! Whether or not an entry is uploaded depends on the sampling configuration. Each type of entry can have a different
//! level of configuration with device config and reboots always being uploaded. All other will be uploaded based on
//! the below rules:
//!
//! +=================+=====+=====+========+======+
//! |    MAR Type     | Off | Low | Medium | High |
//! +=================+=====+=====+========+======+
//! | heartbeat       |     |     | x      | x    |
//! +-----------------+-----+-----+--------+------+
//! | daily-heartbeat |     | x   | x      | x    |
//! +-----------------+-----+-----+--------+------+
//! | session         |     |     | x      | x    |
//! +-----------------+-----+-----+--------+------+
//! | attributes      |     |     | x      | x    |
//! +-----------------+-----+-----+--------+------+
//! | coredump        |     |     | x      | x    |
//! +-----------------+-----+-----+--------+------+
//! | logs            |     |     | x      | x    |
//! +-----------------+-----+-----+--------+------+
//! | CDR             |     |     | x      | x    |
//! +-----------------+-----+-----+--------+------+

use std::fs::{remove_dir_all, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use eyre::{eyre, Context, Result};
use itertools::Itertools;
use log::{trace, warn};

use crate::{
    config::{Resolution, Sampling},
    mar::{MarEntry, Metadata},
    metrics::MetricReportType,
    network::NetworkClient,
    util::zip::{zip_stream_len_empty, zip_stream_len_for_file, ZipEncoder, ZipEntryInfo},
};

/// Collect all valid MAR entries, upload them and delete them on success.
///
/// Returns the number of MAR entries that were uploaded.
///
/// This function will not do anything with invalid MAR entries (we assume they are "under construction").
pub fn collect_and_upload(
    mar_staging: &Path,
    client: &impl NetworkClient,
    max_zip_size: usize,
    sampling: Sampling,
) -> Result<usize> {
    let mut entries = MarEntry::iterate_from_container(mar_staging)?
        // Apply fleet sampling to the MAR entries
        .filter(|entry_result| match entry_result {
            Ok(entry) => should_upload(&entry.manifest.metadata, &sampling),
            _ => true,
        });

    upload_mar_entries(&mut entries, client, max_zip_size, |included_entries| {
        trace!("Uploaded {:?} - deleting...", included_entries);
        included_entries.iter().for_each(|f| {
            let _ = remove_dir_all(f);
        })
    })
}

/// Given the current sampling configuration determine if the given MAR entry should be uploaded.
fn should_upload(metadata: &Metadata, sampling: &Sampling) -> bool {
    match metadata {
        Metadata::DeviceAttributes { .. } => sampling.monitoring_resolution >= Resolution::Normal,
        Metadata::DeviceConfig { .. } => true, // Always upload device config
        Metadata::ElfCoredump { .. } => sampling.debugging_resolution >= Resolution::Normal,
        Metadata::LinuxHeartbeat { .. } => sampling.monitoring_resolution >= Resolution::Normal,
        Metadata::LinuxMetricReport { report_type, .. } => match report_type {
            MetricReportType::Heartbeat => sampling.monitoring_resolution >= Resolution::Normal,
            MetricReportType::Session(_) => sampling.monitoring_resolution >= Resolution::Normal,
            MetricReportType::DailyHeartbeat => sampling.monitoring_resolution >= Resolution::Low,
        },
        Metadata::LinuxLogs { .. } => sampling.logging_resolution >= Resolution::Normal,
        Metadata::LinuxReboot { .. } => true, // Always upload reboots
        Metadata::LinuxMemfaultWatch { exit_code, .. } => {
            let is_crash = exit_code != &0;
            if is_crash {
                sampling.debugging_resolution >= Resolution::Normal
                    || sampling.logging_resolution >= Resolution::Normal
            } else {
                sampling.logging_resolution >= Resolution::Normal
            }
        }
        Metadata::CustomDataRecording { .. } => sampling.debugging_resolution >= Resolution::Normal,
    }
}

/// Describes the contents for a single MAR file to upload.
pub struct MarZipContents {
    /// All the MAR entry directories to to be included in this file.
    pub entry_paths: Vec<PathBuf>,
    /// All the ZipEntryInfos to be included in this file.
    pub zip_infos: Vec<ZipEntryInfo>,
}

/// Gather MAR entries and associated ZipEntryInfos, consuming items from the iterator.
///
/// Return a list of MarZipContents, each containing the list of folders that are included in the
/// zip (and can be deleted after upload) and the list of ZipEntryInfos.
/// Invalid folders will not trigger an error and they will not be included in the returned lists.
pub fn gather_mar_entries_to_zip(
    entries: &mut impl Iterator<Item = Result<MarEntry>>,
    max_zip_size: usize,
) -> Vec<MarZipContents> {
    let entry_paths_with_zip_infos = entries.filter_map(|entry_result| match entry_result {
        Ok(entry) => {
            trace!("Adding {:?}", &entry.path);
            let zip_infos: Option<Vec<ZipEntryInfo>> = (&entry)
                .try_into()
                .wrap_err_with(|| format!("Unable to add entry {}.", &entry.path.display()))
                .ok();
            let entry_and_infos: Option<(PathBuf, Vec<ZipEntryInfo>)> =
                zip_infos.map(|infos| (entry.path, infos));
            entry_and_infos
        }
        Err(e) => {
            warn!("Invalid folder in MAR staging: {:?}", e);
            None
        }
    });

    let mut zip_size = zip_stream_len_empty();
    let mut zip_file_index: usize = 0;
    let grouper = entry_paths_with_zip_infos.group_by(|(_, zip_infos)| {
        let entry_zipped_size = zip_infos.iter().map(zip_stream_len_for_file).sum::<usize>();
        if zip_size + entry_zipped_size > max_zip_size {
            zip_size = zip_stream_len_empty() + entry_zipped_size;
            zip_file_index += 1;
        } else {
            zip_size += entry_zipped_size;
        }
        zip_file_index
    });

    grouper
        .into_iter()
        .map(|(_zip_file_index, group)| {
            // Convert from Vec<(PathBuf, Vec<ZipEntryInfo>)> to MarZipContents:
            let (entry_paths, zip_infos): (Vec<PathBuf>, Vec<Vec<ZipEntryInfo>>) = group.unzip();
            MarZipContents {
                entry_paths,
                zip_infos: zip_infos
                    .into_iter()
                    .flatten()
                    .collect::<Vec<ZipEntryInfo>>(),
            }
        })
        .collect()
}

impl TryFrom<&MarEntry> for Vec<ZipEntryInfo> {
    type Error = eyre::Error;

    fn try_from(entry: &MarEntry) -> Result<Self> {
        let entry_path = entry.path.clone();
        entry
            .filenames()
            .map(move |filename| {
                let path = entry_path.join(&filename);

                // Open the file to check that it exists and is readable. This is a best effort to avoid
                // starting to upload a MAR file only to find out half way through that a file was not
                // readable. Yes, this is prone to a race condition where it is no longer readable by
                // the time is going to be read by the zip writer, but it is better than nothing.
                let file =
                    File::open(&path).wrap_err_with(|| format!("Error opening {:?}", filename))?;
                drop(file);

                let base = entry_path.parent().ok_or(eyre!("No parent directory"))?;
                ZipEntryInfo::new(path, base)
                    .wrap_err_with(|| format!("Error adding {:?}", filename))
            })
            .collect::<Result<Vec<_>>>()
    }
}

/// Progressively upload the MAR entries. The callback will be called for each batch that is uploaded.
fn upload_mar_entries(
    entries: &mut impl Iterator<Item = Result<MarEntry>>,
    client: &impl NetworkClient,
    max_zip_size: usize,
    callback: fn(entries: Vec<PathBuf>) -> (),
) -> Result<usize> {
    let zip_files = gather_mar_entries_to_zip(entries, max_zip_size);
    let count = zip_files.len();

    for MarZipContents {
        entry_paths,
        zip_infos,
    } in zip_files.into_iter()
    {
        client.upload_mar_file(BufReader::new(ZipEncoder::new(zip_infos)))?;
        callback(entry_paths);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};
    use std::str::FromStr;
    use std::{
        collections::HashMap,
        time::{Duration, SystemTime},
    };

    use crate::reboot::{RebootReason, RebootReasonCode};
    use crate::{
        mar::test_utils::{assert_mar_content_matches, MarCollectorFixture},
        metrics::SessionName,
        network::MockNetworkClient,
    };
    use crate::{
        metrics::{MetricStringKey, MetricValue},
        test_utils::setup_logger,
    };

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

    #[rstest]
    fn zipping_two_entries(_setup_logger: (), mut mar_fixture: MarCollectorFixture) {
        // Add one valid entry so we can verify that this one is readable.
        mar_fixture.create_logentry();
        mar_fixture.create_logentry();

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let mars = gather_mar_entries_to_zip(&mut entries, usize::MAX);

        assert_eq!(mars.len(), 1);
        assert_eq!(mars[0].entry_paths.len(), 2);
        assert_eq!(mars[0].zip_infos.len(), 4); // for each entry: manifest.json + log file
    }

    #[rstest]
    #[case::not_json(MarCollectorFixture::create_entry_with_bogus_json)]
    #[case::unreadable_dir(MarCollectorFixture::create_entry_without_directory_read_permission)]
    #[case::unreadable_manifest(MarCollectorFixture::create_entry_without_manifest_read_permission)]
    fn zipping_with_skipped_entries(
        _setup_logger: (),
        mut mar_fixture: MarCollectorFixture,
        #[case] create_bogus_entry: fn(&mut MarCollectorFixture) -> PathBuf,
    ) {
        create_bogus_entry(&mut mar_fixture);
        // Add one valid entry so we can verify that this one is readable.
        mar_fixture.create_logentry();

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let mars = gather_mar_entries_to_zip(&mut entries, usize::MAX);

        assert_eq!(mars.len(), 1);
        assert_eq!(mars[0].entry_paths.len(), 1);
        assert_eq!(mars[0].zip_infos.len(), 2); // manifest.json + log file
    }

    #[rstest]
    fn zipping_an_unreadable_attachment(_setup_logger: (), mut mar_fixture: MarCollectorFixture) {
        // Add one valid entry so we can verify that this one is readable.
        mar_fixture.create_logentry_with_unreadable_attachment();

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let mars = gather_mar_entries_to_zip(&mut entries, usize::MAX);

        // No MAR should be created because the attachment is unreadable.
        assert_eq!(mars.len(), 0);
    }

    #[rstest]
    fn new_mar_when_size_limit_is_reached(_setup_logger: (), mut mar_fixture: MarCollectorFixture) {
        let max_zip_size = 1024;
        mar_fixture.create_logentry_with_size(max_zip_size / 2);
        mar_fixture.create_logentry_with_size(max_zip_size);
        // Note: the next entry exceeds the size limit, but it is still added to a MAR of its own:
        mar_fixture.create_logentry_with_size(max_zip_size * 2);

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let mars = gather_mar_entries_to_zip(&mut entries, max_zip_size as usize);

        // 3 MARs should be created because the size limit was reached after every entry:
        assert_eq!(mars.len(), 3);
        for contents in mars {
            assert_eq!(contents.entry_paths.len(), 1);
            assert_eq!(contents.zip_infos.len(), 2); // for each entry: manifest.json + log file
        }
    }

    #[rstest]
    fn uploading_empty_list(
        _setup_logger: (),
        client: MockNetworkClient,
        mar_fixture: MarCollectorFixture,
    ) {
        // We do not set an expectation on client => it will panic if client.upload_mar is called
        collect_and_upload(
            &mar_fixture.mar_staging,
            &client,
            usize::MAX,
            Sampling {
                debugging_resolution: Resolution::Normal,
                logging_resolution: Resolution::Normal,
                monitoring_resolution: Resolution::Normal,
            },
        )
        .unwrap();
    }

    #[rstest]
    #[case::off(Resolution::Off, false)]
    #[case::low(Resolution::Low, false)]
    #[case::normal(Resolution::Normal, true)]
    #[case::high(Resolution::High, true)]
    fn uploading_logs(
        #[case] resolution: Resolution,
        #[case] should_upload: bool,
        _setup_logger: (),
        client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        mar_fixture.create_logentry();

        let expected_files =
            should_upload.then(|| vec!["<entry>/manifest.json", "<entry>/system.log"]);
        let sampling_config = Sampling {
            debugging_resolution: Resolution::Off,
            logging_resolution: resolution,
            monitoring_resolution: Resolution::Off,
        };
        upload_and_verify(mar_fixture, client, sampling_config, expected_files);
    }

    #[rstest]
    #[case::off(Resolution::Off, false)]
    #[case::low(Resolution::Low, false)]
    #[case::normal(Resolution::Normal, true)]
    #[case::high(Resolution::High, true)]
    fn uploading_device_attributes(
        #[case] resolution: Resolution,
        #[case] should_upload: bool,
        _setup_logger: (),
        client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        mar_fixture.create_device_attributes_entry(vec![], SystemTime::now());

        let sampling_config = Sampling {
            debugging_resolution: Resolution::Off,
            logging_resolution: Resolution::Off,
            monitoring_resolution: resolution,
        };
        let expected_files = should_upload.then(|| vec!["<entry>/manifest.json"]);
        upload_and_verify(mar_fixture, client, sampling_config, expected_files);
    }

    #[rstest]
    // Verify that reboots are always uploaded
    #[case::off(Resolution::Off, true)]
    #[case::low(Resolution::Low, true)]
    #[case::normal(Resolution::Normal, true)]
    #[case::high(Resolution::High, true)]
    fn uploading_reboots(
        #[case] resolution: Resolution,
        #[case] should_upload: bool,
        _setup_logger: (),
        client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        mar_fixture.create_reboot_entry(RebootReason::Code(RebootReasonCode::Unknown));

        let sampling_config = Sampling {
            debugging_resolution: resolution,
            logging_resolution: Resolution::Off,
            monitoring_resolution: Resolution::Off,
        };
        let expected_files = should_upload.then(|| vec!["<entry>/manifest.json"]);
        upload_and_verify(mar_fixture, client, sampling_config, expected_files);
    }

    #[rstest]
    // Verify that CDRs are uploaded based on the debugging resolution
    #[case::off(Resolution::Off, false)]
    #[case::low(Resolution::Low, false)]
    #[case::normal(Resolution::Normal, true)]
    #[case::high(Resolution::High, true)]
    fn uploading_custom_data_recordings(
        #[case] resolution: Resolution,
        #[case] should_upload: bool,
        _setup_logger: (),
        client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        let data = vec![1, 3, 3, 7];
        mar_fixture.create_custom_data_recording_entry(data);

        let sampling_config = Sampling {
            debugging_resolution: resolution,
            logging_resolution: Resolution::Off,
            monitoring_resolution: Resolution::Off,
        };
        let expected_files = should_upload.then(|| vec!["<entry>/data", "<entry>/manifest.json"]);
        upload_and_verify(mar_fixture, client, sampling_config, expected_files);
    }

    #[rstest]
    // Heartbeat cases
    #[case::heartbeat_off(MetricReportType::Heartbeat, Resolution::Off, false)]
    #[case::heartbeat_low(MetricReportType::Heartbeat, Resolution::Low, false)]
    #[case::heartbeat_normal(MetricReportType::Heartbeat, Resolution::Normal, true)]
    #[case::heartbeat_high(MetricReportType::Heartbeat, Resolution::High, true)]
    // Daily heartbeat cases
    #[case::daily_heartbeat_off(MetricReportType::DailyHeartbeat, Resolution::Off, false)]
    #[case::daily_heartbeat_low(MetricReportType::DailyHeartbeat, Resolution::Low, true)]
    #[case::daily_heartbeat_normal(MetricReportType::DailyHeartbeat, Resolution::Normal, true)]
    #[case::daily_heartbeat_high(MetricReportType::DailyHeartbeat, Resolution::High, true)]
    // Session cases
    #[case::session_off(
        MetricReportType::Session(SessionName::from_str("test").unwrap()),
        Resolution::Off,
        false
    )]
    #[case::session_low(
        MetricReportType::Session(SessionName::from_str("test").unwrap()),
        Resolution::Low,
        false
    )]
    #[case::session_normal(
        MetricReportType::Session(SessionName::from_str("test").unwrap()),
        Resolution::Normal,
        true
    )]
    #[case::session_high(
        MetricReportType::Session(SessionName::from_str("test").unwrap()),
        Resolution::High,
        true
    )]
    fn uploading_metric_reports(
        #[case] report_type: MetricReportType,
        #[case] resolution: Resolution,
        #[case] should_upload: bool,
        _setup_logger: (),
        client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        let duration = Duration::from_secs(1);
        let metrics: HashMap<MetricStringKey, MetricValue> = vec![(
            MetricStringKey::from_str("foo").unwrap(),
            MetricValue::Number(1.0),
        )]
        .into_iter()
        .collect();

        mar_fixture.create_metric_report_entry(metrics, duration, report_type);

        let sampling_config = Sampling {
            debugging_resolution: Resolution::Off,
            logging_resolution: Resolution::Off,
            monitoring_resolution: resolution,
        };
        let expected_files = should_upload.then(|| vec!["<entry>/manifest.json"]);
        upload_and_verify(mar_fixture, client, sampling_config, expected_files);
    }

    fn upload_and_verify(
        mar_fixture: MarCollectorFixture,
        mut client: MockNetworkClient,
        sampling_config: Sampling,
        expected_files: Option<Vec<&'static str>>,
    ) {
        if let Some(expected_files) = expected_files {
            client
                .expect_upload_mar_file::<BufReader<ZipEncoder>>()
                .withf(move |buf_reader| {
                    let zip_encoder = buf_reader.get_ref();
                    assert_mar_content_matches(zip_encoder, expected_files.clone())
                })
                .once()
                .returning(|_| Ok(()));
        }
        collect_and_upload(
            &mar_fixture.mar_staging,
            &client,
            usize::MAX,
            sampling_config,
        )
        .unwrap();
    }

    #[fixture]
    fn client() -> MockNetworkClient {
        MockNetworkClient::default()
    }

    #[fixture]
    fn mar_fixture() -> MarCollectorFixture {
        MarCollectorFixture::new()
    }
}
