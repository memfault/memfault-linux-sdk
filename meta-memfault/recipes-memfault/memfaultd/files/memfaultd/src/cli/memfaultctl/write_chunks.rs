//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{cli::memfaultd_client::MemfaultdClient, config::Config};

pub use crate::chunk_relay::ChunksEncoding;
use argh::FromArgValue;
use eyre::Result;
use std::fmt;

const BASE64: &str = "base64";
const HEX: &str = "hex";
const BIN: &str = "bin";
const SDK_DATA_EXPORT: &str = "sdk_data_export";

impl FromArgValue for ChunksEncoding {
    fn from_arg_value(value: &str) -> Result<Self, String> {
        match value {
            BASE64 => Ok(Self::Base64),
            HEX => Ok(Self::Hex),
            BIN => Ok(Self::Bin),
            SDK_DATA_EXPORT => Ok(Self::SdkDataExport),
            _ => Err(format!(
                "Invalid chunks encoding: {}. Expected one of: base64, hex, bin, sdk_data_export",
                value
            )),
        }
    }
}
#[cfg(feature = "chunks-relay")]
impl fmt::Display for ChunksEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Base64 => write!(f, "{}", BASE64),
            Self::Hex => write!(f, "{}", HEX),
            Self::Bin => write!(f, "{}", BIN),
            Self::SdkDataExport => write!(f, "{}", SDK_DATA_EXPORT),
        }
    }
}

pub fn write_chunks(
    config: &Config,
    project_key: Option<String>,
    device_serial: Option<String>,
    chunks: Vec<String>,
) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    client.post_chunks(project_key, device_serial, chunks)
}
