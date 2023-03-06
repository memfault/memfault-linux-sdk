//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path;

use chrono::Utc;
use eyre::Result;
#[cfg(test)]
use mockall::automock;

use crate::config::Config;

pub mod client;
mod requests;

#[cfg_attr(test, automock)]
pub trait NetworkClient {
    /// Patch the list of attributes for current device.
    fn patch_attributes(&self, timestamp: chrono::DateTime<Utc>, json: &str) -> Result<()>;

    /// Post a new event to Memfault.
    fn post_event(&self, event: &str) -> Result<()>;

    /// Upload a coredump file to Memfault.
    fn upload_coredump(&self, path: &path::Path, gzipped: bool) -> Result<()>;

    /// Upload a marfile to Memfault
    fn upload_marfile(&self, file: &path::Path) -> Result<()>;
}

/// Internal representation of what is needed to talk to the backend.
pub struct NetworkConfig {
    pub project_key: String,
    pub base_url: String,
    pub device_id: String,
    pub hardware_version: String,
    pub software_version: String,
    pub software_type: String,
}

impl From<&Config> for NetworkConfig {
    fn from(config: &Config) -> Self {
        NetworkConfig {
            project_key: config.config_file.project_key.clone(),
            device_id: config.device_info.device_id.clone(),
            base_url: config.config_file.base_url.clone(),
            hardware_version: config.device_info.hardware_version.clone(),
            software_type: config.config_file.software_type.clone(),
            software_version: config.config_file.software_version.clone(),
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
        }
    }
}
