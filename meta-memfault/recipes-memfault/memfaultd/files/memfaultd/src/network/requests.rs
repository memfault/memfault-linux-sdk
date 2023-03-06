//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use super::NetworkConfig;
use serde::{Deserialize, Serialize};

/// Device metadata required to prepare and commit uploads.
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadDeviceMetadata<'a> {
    device_serial: &'a str,
    hardware_version: &'a str,
    software_version: &'a str,
    software_type: &'a str,
}

impl<'a> From<&'a NetworkConfig> for UploadDeviceMetadata<'a> {
    fn from(config: &'a NetworkConfig) -> Self {
        UploadDeviceMetadata {
            device_serial: config.device_id.as_str(),
            hardware_version: config.hardware_version.as_str(),
            software_type: config.software_type.as_str(),
            software_version: config.software_version.as_str(),
        }
    }
}

/// Request body to prepare an upload
#[derive(Serialize, Debug)]
pub struct UploadPrepareRequest<'a> {
    content_encoding: Option<&'static str>,
    size: usize,
    device: UploadDeviceMetadata<'a>,
}

impl<'a> UploadPrepareRequest<'a> {
    pub fn prepare(
        config: &'a NetworkConfig,
        filesize: usize,
        gzipped: bool,
    ) -> UploadPrepareRequest<'a> {
        UploadPrepareRequest {
            content_encoding: if gzipped { Some("gzip") } else { None },
            size: filesize,
            device: UploadDeviceMetadata::from(config),
        }
    }
}

/// Response for prepare-upload request
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadPrepareResponse {
    pub data: UploadPrepareResponseData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UploadPrepareResponseData {
    pub upload_url: String,
    pub token: String,
}

/// Request body to commit an upload
#[derive(Serialize, Debug)]
pub struct UploadCommitRequest<'a, 'b> {
    file: UploadCommitRequestFile<'b>,
    device: UploadDeviceMetadata<'a>,
}

#[derive(Serialize, Deserialize, Debug)]
struct UploadCommitRequestFile<'a> {
    token: &'a str,
}

impl<'config, 'token> UploadCommitRequest<'config, 'token> {
    pub fn prepare(config: &'config NetworkConfig, token: &'token str) -> Self {
        Self {
            file: UploadCommitRequestFile { token },
            device: UploadDeviceMetadata::from(config),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct PreparedFile<'a> {
    token: &'a str,
}

#[derive(Serialize, Debug)]
pub struct MarUploadMetadata<'a> {
    device_serial: &'a str,
    file: PreparedFile<'a>,
    hardware_version: &'a str,
    software_type: &'a str,
    software_version: &'a str,
}

impl<'a> MarUploadMetadata<'a> {
    pub fn prepare(config: &'a NetworkConfig, token: &'a str) -> Self {
        Self {
            device_serial: &config.device_id,
            hardware_version: &config.hardware_version,
            software_type: &config.software_type,
            software_version: &config.software_version,
            file: PreparedFile { token },
        }
    }
}
