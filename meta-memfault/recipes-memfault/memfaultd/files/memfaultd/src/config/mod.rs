//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::{Path, PathBuf};

use crate::util::disk_size::DiskSize;

pub use self::{
    config_file::{JsonConfigs, MemfaultdConfig},
    device_info::{DeviceInfo, DeviceInfoWarning},
};
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
const LOGS_SUBDIRECTORY: &str = "logs";
const MAR_STAGING_SUBDIRECTORY: &str = "mar";

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

    pub fn tmp_dir(&self) -> PathBuf {
        match self.config_file.tmp_dir {
            Some(ref tmp_dir) => tmp_dir.clone(),
            None => self.config_file.persist_dir.clone(),
        }
        .into()
    }

    pub fn tmp_dir_max_size(&self) -> DiskSize {
        DiskSize::new_capacity(self.config_file.tmp_dir_max_usage as u64)
    }

    pub fn tmp_dir_min_headroom(&self) -> DiskSize {
        DiskSize {
            bytes: self.config_file.tmp_dir_min_headroom as u64,
            inodes: self.config_file.tmp_dir_min_inodes as u64,
        }
    }

    pub fn coredumps_path(&self) -> PathBuf {
        self.tmp_dir().join(COREDUMP_SUBDIRECTORY)
    }

    pub fn logs_path(&self) -> PathBuf {
        self.tmp_dir().join(LOGS_SUBDIRECTORY)
    }

    pub fn mar_staging_path(&self) -> PathBuf {
        self.tmp_dir().join(MAR_STAGING_SUBDIRECTORY)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{config::Config, util::path::AbsolutePath};

    #[test]
    fn tmp_dir_defaults_to_persist_dir() {
        let config = Config::test_fixture();

        assert_eq!(config.tmp_dir(), config.config_file.persist_dir);
    }

    #[test]
    fn tmp_folder_set() {
        let mut config = Config::test_fixture();
        let abs_path = PathBuf::from("/my/abs/path");
        config.config_file.tmp_dir = Some(AbsolutePath::try_from(abs_path.clone()).unwrap());

        assert_eq!(config.tmp_dir(), abs_path);
    }
}
