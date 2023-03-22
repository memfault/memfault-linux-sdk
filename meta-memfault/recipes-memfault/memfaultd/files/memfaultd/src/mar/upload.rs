//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::{remove_dir_all, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use eyre::{Context, Result};
use itertools::Itertools;
use log::{trace, warn};

use crate::network::NetworkClient;
use crate::util::zip::{zip_stream_len_empty, zip_stream_len_for_file, ZipEncoder, ZipEntryInfo};

use super::mar_entry::{MarEntry, MarEntryIterator};

/// Collect all valid MAR entries, upload them and delete them on success.
///
/// This function will not do anything with invalid MAR entries (we assume they are "under construction").
pub fn collect_and_upload(
    mar_staging: &Path,
    client: &impl NetworkClient,
    max_zip_size: usize,
) -> Result<()> {
    let mut entries = MarEntry::iterate_from_container(mar_staging)?;
    upload_mar_entries(&mut entries, client, max_zip_size, |included_entries| {
        trace!("Uploaded {:?} - deleting...", included_entries);
        included_entries.iter().for_each(|f| {
            let _ = remove_dir_all(f);
        })
    })?;
    Ok(())
}

/// Describes the contents for a single MAR file to upload.
struct MarZipContents {
    /// All the MAR entry directories to to be included in this file.
    entry_paths: Vec<PathBuf>,
    /// All the ZipEntryInfos to be included in this file.
    zip_infos: Vec<ZipEntryInfo>,
}

/// Gather MAR entries and associated ZipEntryInfos, consuming items from the iterator.
///
/// Return a list of MarZipContents, each containing the list of folders that are included in the
/// zip (and can be deleted after upload) and the list of ZipEntryInfos.
/// Invalid folders will not trigger an error and they will not be included in the returned lists.
fn gather_mar_entries_to_zip(
    entries: &mut MarEntryIterator,
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

                let base = entry_path.parent().unwrap();
                ZipEntryInfo::new(path, base)
                    .wrap_err_with(|| format!("Error adding {:?}", filename))
            })
            .collect::<Result<Vec<_>>>()
    }
}

/// Progressively upload the MAR entries. The callback will be called for each batch that is uploaded.
fn upload_mar_entries(
    entries: &mut MarEntryIterator,
    client: &impl NetworkClient,
    max_zip_size: usize,
    callback: fn(entries: Vec<PathBuf>) -> (),
) -> Result<()> {
    for MarZipContents {
        entry_paths,
        zip_infos,
    } in gather_mar_entries_to_zip(entries, max_zip_size)
    {
        client.upload_mar_file(BufReader::new(ZipEncoder::new(zip_infos)))?;
        callback(entry_paths);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use crate::test_utils::setup_logger;
    use crate::{
        mar::test_utils::{assert_mar_content_matches, MarCollectorFixture},
        network::MockNetworkClient,
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
        collect_and_upload(&mar_fixture.mar_staging, &client, usize::MAX).unwrap();
    }

    #[rstest]
    fn uploading_mar_list(
        _setup_logger: (),
        mut client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        mar_fixture.create_logentry();
        client
            .expect_upload_mar_file::<BufReader<ZipEncoder>>()
            .withf(|buf_reader| {
                let zip_encoder = buf_reader.get_ref();
                assert_mar_content_matches(
                    zip_encoder,
                    vec!["<entry>/manifest.json", "<entry>/system.log"],
                )
            })
            .once()
            .returning(|_| Ok(()));
        collect_and_upload(&mar_fixture.mar_staging, &client, usize::MAX).unwrap();
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
