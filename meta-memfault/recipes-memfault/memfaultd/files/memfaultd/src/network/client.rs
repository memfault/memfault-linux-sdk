//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::File;
use std::io::Read;
use std::path;
use std::str;

use eyre::eyre;
use eyre::Context;
use eyre::Result;
use log::{debug, trace};
use reqwest::blocking;
use reqwest::blocking::Body;
use reqwest::header;

use crate::retriable_error::RetriableError;
use crate::util::io::StreamLen;
use crate::util::string::Ellipsis;

use super::requests::UploadPrepareRequest;
use super::requests::UploadPrepareResponse;
use super::requests::{DeviceConfigRequest, MarUploadMetadata};
use super::requests::{DeviceConfigResponse, UploadCommitRequest};
use super::NetworkClient;
use super::NetworkConfig;

/// Memfault Network client
pub struct NetworkClientImpl {
    client: blocking::Client,
    /// A separate client for upload to file storage
    file_upload_client: blocking::Client,
    config: NetworkConfig,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    POST,
}

impl NetworkClientImpl {
    pub fn new(config: NetworkConfig) -> Result<Self> {
        let headers = [
            (
                header::ACCEPT,
                header::HeaderValue::from_static("application/json"),
            ),
            (
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ),
            (
                header::HeaderName::from_static("memfault-project-key"),
                header::HeaderValue::from_str(&config.project_key)?,
            ),
            (
                header::CONTENT_ENCODING,
                header::HeaderValue::from_static("utf-8"),
            ),
        ]
        .into_iter()
        .collect();

        let client = blocking::ClientBuilder::new()
            .default_headers(headers)
            .build()?;

        Ok(NetworkClientImpl {
            client,
            file_upload_client: blocking::Client::new(),
            config,
        })
    }

    fn good_response_or_error(response: blocking::Response) -> Result<blocking::Response> {
        // Map status code to an error
        let status = response.status();
        match status.as_u16() {
            200..=299 => Ok(response),
            // Server errors are expected to be temporary and will be retried later
            500..=599 => Err(RetriableError::ServerError {
                status_code: status.as_u16(),
            }
            .into()),
            // Any other error (404, etc) will be considered fatal and will not be retried.
            // HTTP client errors (4xx) are not expected to happen in normal operation, but can
            // occur due to misconfiguration/integration issues. Log the first 1KB of the response
            // body to help with debugging:
            _ => {
                let mut response_text = response.text().unwrap_or_else(|_| "???".into());
                // Limit the size of the response text to avoid filling up the log:
                response_text.truncate_with_ellipsis(1024);
                Err(eyre!(
                    "Unexpected server response: {} {}",
                    status.as_u16(),
                    response_text
                ))
            }
        }
    }

    /// Send a request to Memfault backend
    fn fetch(&self, method: Method, endpoint: &str, payload: &str) -> Result<blocking::Response> {
        let url = format!("{}{}", self.config.base_url, endpoint);
        debug!(
            "{:?} {} - Payload {} bytes\n{:?}",
            method,
            url,
            payload.len(),
            payload
        );
        let response = self
            .client
            .request(
                match method {
                    Method::POST => reqwest::Method::POST,
                },
                url,
            )
            .body(payload.to_owned())
            .send()
            // "send(): This method fails if there was an error while sending request, redirect loop was detected or redirect limit was exhausted."
            // All kinds of errors here are considered "recoverable" and will be retried.
            .map_err(|e| RetriableError::NetworkError { source: e })?;
        debug!(
            "  Response status {} - Size {:?}",
            response.status(),
            response.content_length(),
        );
        Self::good_response_or_error(response)
    }

    /// Upload a file to S3 and return the file token
    fn prepare_and_upload<R: Read + Send + 'static>(
        &self,
        file: BodyAdapter<R>,
        gzipped: bool,
    ) -> Result<String> {
        let prepare_request =
            UploadPrepareRequest::prepare(&self.config, file.size as usize, gzipped);

        let prepare_response = self
            .fetch(
                Method::POST,
                "/api/v0/upload",
                &serde_json::to_string(&prepare_request)?,
            )?
            .json::<UploadPrepareResponse>()
            .wrap_err("Prepare upload error")?;

        trace!("Upload prepare response: {:?}", prepare_response);

        self.put_file(
            &prepare_response.data.upload_url,
            file,
            if gzipped { Some("gzip") } else { None },
        )
        .wrap_err("Storage upload error")?;
        debug!("Successfully transmitted file");

        Ok(prepare_response.data.token)
    }

    fn put_file<R: Read + Send + 'static>(
        &self,
        url: &str,
        file: BodyAdapter<R>,
        content_encoding: Option<&str>,
    ) -> Result<()> {
        let mut req = self.file_upload_client.put(url);

        if let Some(content_encoding) = content_encoding {
            trace!("Adding content-encoding header");
            req = req.header(header::CONTENT_ENCODING, content_encoding);
        }

        trace!("Uploading file to {}", url);
        let body: Body = file.into();
        let r = req.body(body).send()?;
        Self::good_response_or_error(r).and(Ok(()))
    }
}

impl NetworkClient for NetworkClientImpl {
    fn post_event(&self, payload: &str) -> Result<()> {
        self.fetch(Method::POST, "/api/v0/events", payload)
            .and(Ok(()))
    }

    fn upload_coredump(&self, filepath: &path::Path, gzipped: bool) -> Result<()> {
        let token = self.prepare_and_upload(File::open(filepath)?.try_into()?, gzipped)?;

        let commit = UploadCommitRequest::prepare(&self.config, &token);
        self.fetch(
            Method::POST,
            "/api/v0/upload/elf_coredump",
            &serde_json::to_string(&commit)?,
        )
        .and(Ok(()))
        .wrap_err("Coredump commit error")
    }

    fn upload_mar_file<F: Read + StreamLen + Send + 'static>(&self, file: F) -> Result<()> {
        let token = self.prepare_and_upload(file.into(), false)?;

        let mar_upload = MarUploadMetadata::prepare(&self.config, &token);
        self.fetch(
            Method::POST,
            "/api/v0/upload/mar",
            &serde_json::to_string(&mar_upload)?,
        )
        .wrap_err("MAR Upload Error")
        .and(Ok(()))
    }

    fn fetch_device_config(&self) -> Result<super::requests::DeviceConfigResponse> {
        let request = DeviceConfigRequest::from(&self.config);
        self.fetch(
            Method::POST,
            "/api/v0/device-config",
            &serde_json::to_string(&request)?,
        )?
        .json::<DeviceConfigResponse>()
        .wrap_err("Fetch device-config error")
    }
}

/// Small helper to adapt a Read/File into a Body.
/// Note it's not possible to directly write: impl<T: Read + ...> From<T> for Body { ... }
/// because of orphan rules. See https://doc.rust-lang.org/error_codes/E0210.html
struct BodyAdapter<R: Read + Send> {
    reader: R,
    size: u64,
}

impl<R: Read + StreamLen + Send> From<R> for BodyAdapter<R> {
    fn from(reader: R) -> Self {
        let size = reader.stream_len();
        Self { reader, size }
    }
}

impl TryFrom<File> for BodyAdapter<File> {
    type Error = std::io::Error;

    fn try_from(file: File) -> Result<Self, Self::Error> {
        let size = file.metadata()?.len();
        Ok(Self { reader: file, size })
    }
}

impl<T: Read + Send + 'static> From<BodyAdapter<T>> for Body {
    fn from(wrapper: BodyAdapter<T>) -> Self {
        Body::sized(wrapper.reader, wrapper.size)
    }
}
