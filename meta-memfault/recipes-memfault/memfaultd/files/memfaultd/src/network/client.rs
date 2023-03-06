//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs;
use std::path;
use std::str;

use chrono::Utc;
use eyre::eyre;
use eyre::Context;
use eyre::Result;
use log::{debug, trace};
use reqwest::blocking;
use reqwest::header;

use crate::retriable_error::RetriableError;

use super::requests::MarUploadMetadata;
use super::requests::UploadCommitRequest;
use super::requests::UploadPrepareRequest;
use super::requests::UploadPrepareResponse;
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
    PATCH,
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
            // In testing we capture the full response.
            _ if cfg!(test) => Err(eyre!(
                "Unexpected server response: {} {}",
                status.as_u16(),
                response.text()?
            )),
            _ => Err(eyre!("Unexpected server response: {}", status.as_u16())),
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
                    Method::PATCH => reqwest::Method::PATCH,
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
    fn prepare_and_upload(&self, file: &path::Path, gzipped: bool) -> Result<String> {
        let metadata = std::fs::metadata(file)?;

        if !metadata.is_file() {
            return Err(eyre!("{} is not a file.", file.display()));
        }

        let prepare_request =
            UploadPrepareRequest::prepare(&self.config, metadata.len() as usize, gzipped);

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

    fn put_file(
        &self,
        url: &str,
        filepath: &path::Path,
        content_encoding: Option<&str>,
    ) -> Result<()> {
        let file = fs::File::open(filepath)?;
        let mut req = self.file_upload_client.put(url);

        if let Some(content_encoding) = content_encoding {
            trace!("Adding content-encoding header");
            req = req.header(header::CONTENT_ENCODING, content_encoding);
        }

        trace!("Uploading {} to {}", filepath.display(), url);
        let r = req.body(file).send()?;
        Self::good_response_or_error(r).and(Ok(()))
    }
}

impl NetworkClient for NetworkClientImpl {
    fn patch_attributes(&self, timestamp: chrono::DateTime<Utc>, json: &str) -> Result<()> {
        let path = format!(
            "/api/v0/attributes?device_serial={}&captured_date={}",
            self.config.device_id,
            urlencoding::encode(&timestamp.to_rfc3339())
        );
        self.fetch(Method::PATCH, &path, json).and(Ok(()))
    }

    fn post_event(&self, payload: &str) -> Result<()> {
        self.fetch(Method::POST, "/api/v0/events", payload)
            .and(Ok(()))
    }

    fn upload_coredump(&self, file: &path::Path, gzipped: bool) -> Result<()> {
        let token = self.prepare_and_upload(file, gzipped)?;

        let commit = UploadCommitRequest::prepare(&self.config, &token);
        self.fetch(
            Method::POST,
            "/api/v0/upload/elf_coredump",
            &serde_json::to_string(&commit)?,
        )
        .and(Ok(()))
        .wrap_err("Coredump commit error")
    }

    fn upload_marfile(&self, file: &path::Path) -> Result<()> {
        let token = self.prepare_and_upload(file, false)?;

        let mar_upload = MarUploadMetadata::prepare(&self.config, &token);
        self.fetch(
            Method::POST,
            "/api/v0/upload/mar",
            &serde_json::to_string(&mar_upload)?,
        )
        .wrap_err("MAR Upload Error")
        .and(Ok(()))
    }
}
