//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Context, Result};
use std::io::Read;

use reqwest::{blocking::Client, header::ACCEPT, StatusCode};

use crate::{
    config::Config,
    mar::{ExportFormat, EXPORT_MAR_URL},
};

/// Client to Memfaultd localhost HTTP API
pub struct MemfaultdClient {
    base_url: String,
    client: Client,
}

pub struct DeleteToken(String);

pub enum ExportGetResponse {
    Data {
        delete_token: DeleteToken,
        data: Box<dyn Read>,
    },
    NoData,
}

pub enum ExportDeleteResponse {
    Ok,
    ErrorWrongDeleteToken,
    Error404,
}

impl MemfaultdClient {
    pub fn from_config(config: &Config) -> Self {
        MemfaultdClient {
            client: Client::new(),
            base_url: format!("http://{}", config.config_file.http_server.bind_address),
        }
    }

    pub fn export_get(&self, format: &ExportFormat) -> Result<ExportGetResponse> {
        let r = self
            .client
            .get(format!("{}{}", self.base_url, EXPORT_MAR_URL))
            .header(ACCEPT, format.to_content_type())
            .send()
            .wrap_err_with(|| {
                eyre!(format!(
                    "Error fetching {}/{}",
                    self.base_url, EXPORT_MAR_URL
                ))
            })?;
        match r.status() {
            StatusCode::OK => Ok(ExportGetResponse::Data {
                delete_token: DeleteToken(
                    r.headers()
                        .iter()
                        .find(|h| h.0.as_str() == "etag")
                        .ok_or(eyre!("No ETag header included on the response"))
                        .map(|etag| etag.1.to_str())??
                        .trim_matches('"')
                        .to_owned(),
                ),
                data: Box::new(r),
            }),
            StatusCode::NO_CONTENT => Ok(ExportGetResponse::NoData),
            StatusCode::NOT_ACCEPTABLE => Err(eyre!("Requested format not supported")),
            _ => Err(eyre!("Unexpected status code {}", r.status().as_u16())),
        }
    }

    pub fn export_delete(&self, delete_token: DeleteToken) -> Result<ExportDeleteResponse> {
        let r = self
            .client
            .delete(format!("{}{}", self.base_url, EXPORT_MAR_URL))
            .header("If-Match", delete_token.0)
            .send()?;
        match r.status() {
            StatusCode::NO_CONTENT => Ok(ExportDeleteResponse::Ok),
            StatusCode::PRECONDITION_FAILED => Ok(ExportDeleteResponse::ErrorWrongDeleteToken),
            StatusCode::NOT_FOUND => Ok(ExportDeleteResponse::Error404),
            _ => Err(eyre!(format!(
                "Unexpected status code {}",
                r.status().as_u16()
            ))),
        }
    }
}
