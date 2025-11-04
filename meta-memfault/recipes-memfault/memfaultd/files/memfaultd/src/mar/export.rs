//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{
    http_server::{ConvenientHeader, HttpHandler, HttpHandlerResult},
    mar::{
        gather_mar_entries_to_zip, Chunk, ChunkMessage, ChunkWrapper, ExportFormat, MarEntry,
        MarZipContents,
    },
    util::{io::StreamLen, zip::ZipEncoder},
};
use eyre::{eyre, Result};
use log::{debug, trace, warn};
use std::{
    collections::hash_map::DefaultHasher,
    fs::remove_dir_all,
    io::BufReader,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use std::{hash::Hasher, os::unix::prelude::OsStrExt};
use tiny_http::{Header, Method, Request, Response, ResponseBox, StatusCode};

pub const EXPORT_MAR_URL: &str = "/v1/export";

/// Information on the most recent export that was proposed to a client.
/// We will keep offering the same content until we receive a DELETE call.
struct ExportInfo {
    content: MarZipContents,
    hash: String,
}
impl ExportInfo {
    fn new(content: MarZipContents) -> Self {
        let mut hasher = DefaultHasher::new();
        for p in content.entry_paths.iter() {
            hasher.write_usize(p.as_os_str().len());
            hasher.write(p.as_os_str().as_bytes());
        }
        let hash = hasher.finish().to_string();
        ExportInfo { content, hash }
    }

    /// Verifies if the files are still available on disk
    fn is_valid(&self) -> bool {
        self.content.entry_paths.iter().all(|p| p.exists())
    }
}

#[derive(Clone)]
/// An `HttpHandler` to manage exporting memfaultd data to local clients.
///
/// Clients should first make a GET request to download the data and write it to
/// disk. When the file has safely transmitted, they can all DELETE to delete
/// the data secured in the MAR staging directory.
///
/// For additional security (if multiple clients are accidentally
/// reading/deleting concurrently), we recommend using the If-Match header when
/// calling DELETE and passing the ETag returned by the GET call. This will
/// confirm that the data being deleted is the data that was just saved.
pub struct MarExportHandler {
    mar_staging: PathBuf,
    current_export: Arc<Mutex<Option<ExportInfo>>>,
}

impl MarExportHandler {
    pub fn new(mar_staging: PathBuf) -> Self {
        MarExportHandler {
            mar_staging,
            current_export: Arc::new(Mutex::new(None)),
        }
    }
}

const DEFAULT_MAX_ZIP_FILE: usize = 10 * 1024 * 1024;

impl MarExportHandler {
    /// Looks at data in the mar_staging folder and returns content that should be included in next ZIP download
    fn prepare_next_export(&self) -> Result<Option<ExportInfo>> {
        let mut entries = MarEntry::iterate_from_container(&self.mar_staging)?;

        let zip_files = gather_mar_entries_to_zip(&mut entries, DEFAULT_MAX_ZIP_FILE);
        match zip_files.into_iter().next() {
            Some(zip) => Ok(Some(ExportInfo::new(zip))),
            None => Ok(None),
        }
    }

    fn handle_get_mar(&self, request: &Request) -> Result<ResponseBox> {
        let mut export = self
            .current_export
            .lock()
            .map_err(|e| eyre!("Export Mutex poisoned: {:#}", e))?;

        // If we have already prepared a package, make sure the file still exists on disk - or reset the package.
        if let Some(false) = (export.as_ref()).map(|export| export.is_valid()) {
            *export = None;
        }

        // Prepare a new package if needed
        if export.is_none() {
            *export = self.prepare_next_export()?;
        }

        // Parse the accept-header, return 406 NotAcceptable if the requested format is not supported.
        let accept_header = request.headers().iter().find(|h| h.field.equiv("Accept"));
        let format = match accept_header {
            Some(header) => match ExportFormat::from_accept_header(header.value.as_str()) {
                Ok(format) => format,
                Err(_) => return Ok(Response::empty(406).boxed()),
            },
            None => ExportFormat::default(),
        };

        // If we have data to serve, prime the ZIP encoder and stream the data
        // Otherwise, return 204.
        match &*export {
            Some(export) => match format {
                ExportFormat::Mar => Self::export_as_zip(export),
                ExportFormat::Chunk => Self::export_as_chunk(export),
                ExportFormat::ChunkWrapped => Self::export_as_chunk_wrapped(export),
            },
            None => Ok(Response::empty(204).boxed()),
        }
    }

    fn export_as_zip(export: &ExportInfo) -> Result<ResponseBox> {
        let zip_encoder = ZipEncoder::new(export.content.zip_infos.clone());
        let len = zip_encoder.stream_len();

        Ok(Response::new(
            StatusCode(200),
            vec![
                Header::from_strings("Content-Type", "application/zip")?,
                Header::from_strings("ETag", &format!("\"{}\"", export.hash))?,
            ],
            BufReader::new(zip_encoder),
            Some(len as usize),
            None,
        )
        .boxed())
    }

    fn export_as_chunk(export: &ExportInfo) -> Result<ResponseBox> {
        let zip_encoder = ZipEncoder::new(export.content.zip_infos.clone());

        let chunk_stream = Chunk::new_single(ChunkMessage::new(
            super::chunks::ChunkMessageType::Mar,
            zip_encoder,
        ));

        let len = chunk_stream.stream_len();

        Ok(Response::new(
            StatusCode(200),
            vec![
                Header::from_strings("Content-Type", ExportFormat::Chunk.to_content_type())?,
                Header::from_strings("ETag", &format!("\"{}\"", export.hash))?,
            ],
            BufReader::new(chunk_stream),
            Some(len as usize),
            None,
        )
        .boxed())
    }

    fn export_as_chunk_wrapped(export: &ExportInfo) -> Result<ResponseBox> {
        let zip_encoder = ZipEncoder::new(export.content.zip_infos.clone());

        let chunk = ChunkWrapper::new(Chunk::new_single(ChunkMessage::new(
            super::chunks::ChunkMessageType::Mar,
            zip_encoder,
        )));
        let len = chunk.stream_len();

        Ok(Response::new(
            StatusCode(200),
            vec![
                Header::from_strings("Content-Type", ExportFormat::Chunk.to_content_type())?,
                Header::from_strings("ETag", &format!("\"{}\"", export.hash))?,
            ],
            BufReader::new(chunk),
            Some(len as usize),
            None,
        )
        .boxed())
    }

    fn handle_delete(&self, request: &Request) -> Result<ResponseBox> {
        let mut export_opt = self
            .current_export
            .lock()
            .map_err(|e| eyre!("Mutex poisoned: {:#}", e))?;

        if let Some(export) = export_opt.as_ref() {
            // Optionally, check that the ETag matches (to confirm we are deleting the data client just read).
            if let Some(if_match_header) =
                request.headers().iter().find(|h| h.field.equiv("If-Match"))
            {
                if if_match_header.value != export.hash {
                    debug!(
                        "Delete error - Wrong hash. Got {}, expected {}",
                        if_match_header.value, export.hash
                    );
                    return Ok(Response::from_string("Precondition Failed")
                        .with_status_code(412)
                        .boxed());
                }
            }

            trace!("Deleting MAR entries: {:?}", export.content.entry_paths);
            export.content.entry_paths.iter().for_each(|f| {
                if let Err(e) = remove_dir_all(f) {
                    warn!("Error deleting MAR entry: {} ({})", f.display(), e)
                }
            });
            *export_opt = None;
            Ok(Response::empty(204).boxed())
        } else {
            trace!("Export delete called but no current content to delete.");
            Ok(Response::empty(404).boxed())
        }
    }
}

impl HttpHandler for MarExportHandler {
    // TODO: MFLT-11507 Handle locking the mar_cleaner while we are reading it!
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() == EXPORT_MAR_URL {
            match *request.method() {
                Method::Get => self.handle_get_mar(request).into(),
                Method::Delete => self.handle_delete(request).into(),
                _ => HttpHandlerResult::Response(Response::empty(405).boxed()),
            }
        } else {
            HttpHandlerResult::NotHandled
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::remove_dir_all, str::FromStr};

    use rstest::{fixture, rstest};
    use tiny_http::{Header, ResponseBox, StatusCode, TestRequest};

    use crate::{
        http_server::HttpHandler, mar::test_utils::MarCollectorFixture, util::disk_size::get_size,
    };

    use super::{MarExportHandler, EXPORT_MAR_URL};

    #[rstest]
    fn answer_204_when_empty(mut fixture: Fixture) {
        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(204));
        assert_eq!(r.etag(), None);
    }

    #[rstest]
    fn download_zip(mut fixture: Fixture) {
        fixture.mar_fixture.create_logentry_with_size(512, false);

        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(200));

        r.etag().expect("e-tag header should be included");

        // Files should still be there.
        assert!(fixture.count_mar_inodes() > 0);
    }

    #[rstest]
    fn download_twice(mut fixture: Fixture) {
        fixture.mar_fixture.create_logentry_with_size(512, false);

        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(200));

        // Another GET should yield the same response - even if we have added files in between
        fixture.mar_fixture.create_logentry_with_size(1024, false);
        let r2 = fixture.do_download();

        assert_eq!(r2.status_code(), StatusCode(200));
        assert_eq!(r.data_length().unwrap(), r2.data_length().unwrap());
        assert_eq!(r.etag().unwrap(), r2.etag().unwrap());
    }

    #[rstest]
    fn download_reset_on_cleanup(mut fixture: Fixture) {
        let log1 = fixture.mar_fixture.create_logentry_with_size(512, false);

        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(200));

        // Simulate mar cleaner removing some files
        remove_dir_all(log1).expect("delete failed");

        // Another GET should yield a new response (because the old files are not available anymore)
        fixture.mar_fixture.create_logentry_with_size(1024, false);
        let r2 = fixture.do_download();

        assert_eq!(r2.status_code(), StatusCode(200));
        assert_ne!(r.data_length().unwrap(), r2.data_length().unwrap());
        assert_ne!(r.etag().unwrap(), r2.etag().unwrap());
    }

    #[rstest]
    fn files_should_be_deleted_with_etag(mut fixture: Fixture) {
        fixture.mar_fixture.create_logentry_with_size(512, false);

        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(200));

        let delete_response = fixture.do_delete(Some(r.etag().unwrap()));
        assert_eq!(delete_response.status_code(), StatusCode(204));

        // Files should have been deleted.
        assert_eq!(fixture.count_mar_inodes(), 0);
    }

    #[rstest]
    fn files_should_be_deleted_without_etag(mut fixture: Fixture) {
        fixture.mar_fixture.create_logentry_with_size(512, false);

        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(200));

        let delete_response = fixture.do_delete(None);
        assert_eq!(delete_response.status_code(), StatusCode(204));

        // Files should have been deleted.
        assert_eq!(fixture.count_mar_inodes(), 0);
    }

    #[rstest]
    fn files_should_not_delete_if_etag_does_not_match(mut fixture: Fixture) {
        fixture.mar_fixture.create_logentry_with_size(512, false);

        let r = fixture.do_download();
        assert_eq!(r.status_code(), StatusCode(200));

        let delete_response = fixture.do_delete(Some("bogus".to_owned()));
        assert_eq!(delete_response.status_code(), StatusCode(412));

        // Files should NOT have been deleted.
        assert!(fixture.count_mar_inodes() > 0);
    }

    #[rstest]
    fn error_404_for_deletes(mut fixture: Fixture) {
        fixture.mar_fixture.create_logentry_with_size(512, false);

        // Not calling download before calling delete

        let delete_response = fixture.do_delete(None);
        assert_eq!(delete_response.status_code(), StatusCode(404));

        // Files should NOT have been deleted.
        assert!(fixture.count_mar_inodes() > 0);
    }

    struct Fixture {
        mar_fixture: MarCollectorFixture,
        handler: MarExportHandler,
    }

    impl Fixture {
        fn do_download(&mut self) -> ResponseBox {
            let r = TestRequest::new()
                .with_method(tiny_http::Method::Get)
                .with_path(EXPORT_MAR_URL);

            self.handler
                .handle_request(&mut r.into())
                .expect("should process the request")
        }

        fn do_delete(&mut self, hash: Option<String>) -> ResponseBox {
            let mut r = TestRequest::new()
                .with_method(tiny_http::Method::Delete)
                .with_path(EXPORT_MAR_URL);

            if let Some(hash) = hash {
                r = r.with_header(Header::from_str(&format!("If-Match: {}", hash)).unwrap())
            }

            self.handler
                .handle_request(&mut r.into())
                .expect("should process the request")
        }

        fn count_mar_inodes(&self) -> usize {
            get_size(&self.mar_fixture.tmp_mar_staging)
                .expect("count mar files")
                .inodes as usize
        }
    }

    #[fixture]
    fn fixture() -> Fixture {
        let mar_fixture = MarCollectorFixture::new();

        Fixture {
            handler: MarExportHandler::new(mar_fixture.tmp_mar_staging.clone()),
            mar_fixture,
        }
    }

    trait ResponseUtils {
        fn etag(&self) -> Option<String>;
    }
    impl ResponseUtils for ResponseBox {
        fn etag(&self) -> Option<String> {
            self.headers()
                .iter()
                .find(|h| h.field.equiv("ETag"))
                .map(|header| header.value.as_str().trim_matches('"').to_string())
        }
    }
}
