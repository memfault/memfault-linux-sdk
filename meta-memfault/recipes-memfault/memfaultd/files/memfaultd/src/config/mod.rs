//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::{Path, PathBuf};

use self::{config_file::MemfaultdConfig, device_info::DeviceInfo};
use eyre::Result;

mod config_file;
mod device_info;

/// Container of the entire memfaultd configuration.
/// Implement `From<Config>` trait to initialize module specific configuration (see `NetworkConfig` for example).
pub struct Config {
    pub device_info: DeviceInfo,
    pub config_file: MemfaultdConfig,
}

// See memfault-core-handler which places coredumps in this directory.
const COREDUMP_SUBDIRECTORY: &str = "core";
const MAR_STAGING_SUBDIRECTORY: &str = "mar_staging";

impl Config {
    pub fn read_from_system(user_config: Option<&Path>) -> Result<Self> {
        let config = MemfaultdConfig::load(user_config)?;

        let (device_info, warnings) = DeviceInfo::load()?;
        warnings.iter().for_each(|w| eprintln!("{}", w));

        Ok(Self {
            device_info,
            config_file: config,
        })
    }

    pub fn coredumps_path(&self) -> PathBuf {
        self.config_file.data_dir.join(COREDUMP_SUBDIRECTORY)
    }

    pub fn mar_staging_path(&self) -> PathBuf {
        self.config_file.data_dir.join(MAR_STAGING_SUBDIRECTORY)
    }
}

#[cfg(test)]
impl Config {
    pub fn test_fixture() -> Self {
        Config {
            device_info: DeviceInfo::test_fixture(),
            config_file: MemfaultdConfig::test_fixture(),
        }
    }
}
