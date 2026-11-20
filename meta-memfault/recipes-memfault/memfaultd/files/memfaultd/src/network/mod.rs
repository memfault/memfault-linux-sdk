//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::Read;
use std::time::Duration;

use eyre::Result;
#[cfg(test)]
use mockall::automock;

use crate::config::Config;
use crate::util::io::StreamLen;

mod client;
pub use client::NetworkClientImpl;

mod requests;
pub use requests::*;

#[cfg_attr(test, automock)]
pub trait NetworkClient {
    /// Upload a MAR file to Memfault
    fn upload_mar_file<F: Read + StreamLen + Send + 'static>(&self, file: F) -> Result<()>;

    /// Fetch DeviceConfig from Memfault.
    fn fetch_device_config(&self) -> Result<DeviceConfigResponse>;
}

/// Internal representation of what is needed to talk to the backend.
#[derive(Clone)]
pub struct NetworkConfig {
    pub project_key: String,
    pub base_url: String,
    pub device_id: String,
    pub hardware_version: String,
    pub software_version: String,
    pub software_type: String,
    pub upload_timeout: Duration,
}

impl From<&Config> for NetworkConfig {
    fn from(config: &Config) -> Self {
        NetworkConfig {
            project_key: config.config_file.project_key.clone(),
            device_id: config.device_info.device_id.clone(),
            base_url: config.config_file.base_url.clone(),
            hardware_version: config.device_info.hardware_version.clone(),
            software_type: config.software_type().to_string(),
            software_version: config.software_version().to_string(),
            upload_timeout: config.config_file.mar.upload_timeout,
        }
    }
}

#[cfg(test)]
impl NetworkConfig {
    pub fn test_fixture() -> Self {
        NetworkConfig {
            project_key: "abcd".to_owned(),
            base_url: "https://devices.memfault.com/".to_owned(),
            device_id: "001".to_owned(),
            hardware_version: "DVT".to_owned(),
            software_version: "1.0.0".to_owned(),
            software_type: "test".to_owned(),
            upload_timeout: Duration::from_secs(30),
        }
    }
}
