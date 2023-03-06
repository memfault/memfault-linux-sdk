//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Context, Result};
use log::{trace, warn};
use std::fs::{remove_dir_all, File};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use zip::{write::FileOptions, ZipWriter};

use crate::network::NetworkClient;

use super::mar_entry::{MarEntry, MarEntryIterator};

/// Collect all valid MAR entries, upload them and delete them on success.
///
/// This function will not do anything with invalid MAR entries (we assume they are "under construction").
pub fn collect_and_upload(mar_staging: &Path, client: &impl NetworkClient) -> Result<()> {
    let mut entries = MarEntry::iterate_from_container(mar_staging)?;
    upload_mar_entries(&mut entries, client, |included_entries| {
        trace!("Uploaded {:?} - deleting...", included_entries);
        included_entries.iter().for_each(|f| {
            let _ = remove_dir_all(f);
        })
    })?;
    Ok(())
}

/// Zip mar entries into the provided file, consuming items from the iterator.
/// To continue zipping, caller can call this function again with the same iterator.
///
/// Return the list of folders that are included in the zip and can be deleted
/// after upload.
/// Will return an error only on write errors. Invalid folders will not trigger
/// an error but they will not be included in the zip or in the returned list.
fn zip_mar_entries<F: Write + Seek>(
    entries: &mut MarEntryIterator,
    file: F,
) -> Result<Vec<PathBuf>> {
    let mut zip = zip::ZipWriter::new(file);

    let mut zipped_entries = vec![];
    for entry_result in entries {
        match entry_result {
            Ok(entry) => {
                trace!("Adding {:?}", entry.path);
                add_entry_to_zip(&mut zip, &entry)
                    .wrap_err_with(|| format!("Unable to add entry {}.", entry.path.display()))?;
                zipped_entries.push(entry.path)
            }
            Err(e) => {
                warn!("Invalid folder in MAR staging: {:?}", e)
            }
        }
    }

    zip.finish()?;
    Ok(zipped_entries)
}

fn add_entry_to_zip<F: Write + Seek>(zip: &mut ZipWriter<F>, entry: &MarEntry) -> Result<()> {
    let options: FileOptions =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let dir_name = entry.uuid.to_string();
    zip.add_directory(&dir_name, options)?;

    // Add the manifest
    add_file_to_zip(&dir_name, zip, &entry.path.join("manifest.json"))
        .wrap_err("Error copying manifest.json")?;
    for file_name in entry.manifest.attachments() {
        add_file_to_zip(&dir_name, zip, &entry.path.join(&file_name))
            .wrap_err(format!("Error copying {:?}", file_name))?;
    }
    Ok(())
}

fn add_file_to_zip<F: Write + Seek>(
    dir_name: &String,
    zip: &mut ZipWriter<F>,
    filepath: &Path,
) -> Result<()> {
    let options: FileOptions =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let filename = filepath
        .file_name()
        .ok_or_else(|| eyre!("Invalid entry resource {}", filepath.display()))?;
    let filename_in_zip = PathBuf::from(&dir_name).join(filename);
    zip.start_file(filename_in_zip.to_string_lossy(), options)?;
    let mut file = File::open(filepath)?;
    std::io::copy(&mut file, zip)?;
    Ok(())
}

/// Progressively upload the MAR entries. The callback will be called for each batch that is uploaded.
fn upload_mar_entries(
    entries: &mut MarEntryIterator,
    client: &impl NetworkClient,
    callback: fn(entries: Vec<PathBuf>) -> (),
) -> Result<()> {
    loop {
        let mut zip = NamedTempFile::new()?;
        let zipped_entries = zip_mar_entries(entries, &mut zip)?;

        if zipped_entries.is_empty() {
            break;
        }
        client.upload_marfile(zip.path())?;
        callback(zipped_entries);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_utils::setup_logger;
    use rstest::{fixture, rstest};
    use tempfile::tempfile;

    use crate::{
        mar::test_utils::{assert_mar_content_matches, MarCollectorFixture},
        network::MockNetworkClient,
        test_utils::SizeLimitedFile,
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
        // create_bogus_entry(&mut mar_fixture);
        // Add one valid entry so we can verify that this one is readable.
        mar_fixture.create_logentry();
        mar_fixture.create_logentry();

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let file = tempfile().unwrap();

        assert_eq!(
            zip_mar_entries(&mut entries, file)
                .expect("zip_mar_entries should not return an error")
                .len(),
            2
        )
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

        let file = tempfile().unwrap();

        assert_eq!(
            zip_mar_entries(&mut entries, file)
                .expect("zip_mar_entries should not return an error for invalid data")
                .len(),
            1
        )
    }

    #[rstest]
    fn zipping_an_unreadable_attachment(_setup_logger: (), mut mar_fixture: MarCollectorFixture) {
        // Add one valid entry so we can verify that this one is readable.
        mar_fixture.create_logentry_with_unreadable_attachment();

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let file = tempfile().unwrap();

        let r = zip_mar_entries(&mut entries, file);

        // We should return an error if the attachment is unreadable.
        assert!(r.is_err());
        assert!(format!("{:?}", r.err().unwrap()).contains("Permission denied"))
    }

    #[rstest]
    fn zipping_into_a_full_drive(_setup_logger: (), mut mar_fixture: MarCollectorFixture) {
        mar_fixture.create_logentry();

        let mut entries = MarEntry::iterate_from_container(&mar_fixture.mar_staging)
            .expect("We should still be able to collect.");

        let file = SizeLimitedFile::new(tempfile().unwrap(), 100);

        let r = zip_mar_entries(&mut entries, file);

        // We should return an error if we are unable to zip because the drive is full
        assert!(r.is_err());
        assert!(format!("{:?}", r.err().unwrap()).contains("limit reached"))
    }

    #[rstest]
    fn uploading_empty_list(
        _setup_logger: (),
        client: MockNetworkClient,
        mar_fixture: MarCollectorFixture,
    ) {
        // We do not set an expectation on client => it will panic if client.upload_mar is called
        collect_and_upload(&mar_fixture.mar_staging, &client).unwrap();
    }

    #[rstest]
    fn uploading_mar_list(
        _setup_logger: (),
        mut client: MockNetworkClient,
        mut mar_fixture: MarCollectorFixture,
    ) {
        mar_fixture.create_logentry();
        client
            .expect_upload_marfile()
            .withf(move |zip| {
                assert_mar_content_matches(
                    zip,
                    vec!["<entry>/", "<entry>/manifest.json", "<entry>/system.log"],
                )
            })
            .once()
            .returning(|_| Ok(()));
        collect_and_upload(&mar_fixture.mar_staging, &client).unwrap();
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
