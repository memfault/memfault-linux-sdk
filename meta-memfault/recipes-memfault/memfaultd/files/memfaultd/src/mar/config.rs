//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::PathBuf;

use crate::{
    config::{Config, PersistStorageConfig},
    mar::Metadata,
};

#[derive(Clone, Debug)]
pub struct MarConfig {
    tmp_dir: PathBuf,
    persist_dir: PathBuf,
    persist_config: Option<PersistStorageConfig>,
}

impl MarConfig {
    pub fn final_staging_path(&self, mar_metadata: &Metadata) -> PathBuf {
        let use_persist_dir =
            self.persist_config
                .as_ref()
                .is_some_and(|config| match mar_metadata {
                    Metadata::LinuxLogs { .. } => config.logs,
                    Metadata::DeviceAttributes { .. } => config.metrics,
                    Metadata::DeviceConfig { .. } => false,
                    Metadata::ElfCoredump { .. } => config.coredumps,
                    Metadata::LinuxReboot { .. } => config.reboots,
                    Metadata::LinuxHeartbeat { .. } => config.metrics,
                    Metadata::LinuxMetricReport { .. } => config.metrics,
                    Metadata::LinuxCustomTrace { .. } => config.coredumps,
                    Metadata::CustomDataRecording { .. } => false,
                    Metadata::Stacktrace { .. } => config.coredumps,
                });

        if use_persist_dir {
            self.persist_dir.clone()
        } else {
            self.tmp_dir.clone()
        }
    }

    pub fn tmp_staging_path(&self) -> PathBuf {
        self.tmp_dir.clone()
    }
}

impl From<&Config> for MarConfig {
    fn from(config: &Config) -> Self {
        Self {
            tmp_dir: config.mar_tmp_staging_path(),
            persist_dir: config.mar_persist_staging_path(),
            persist_config: config.mar_persist_storage_config().copied(),
        }
    }
}

#[cfg(test)]
mod test {
    use rstest::rstest;
    use std::{fs::create_dir_all, path::Path};

    use tempfile::TempDir;

    use super::*;

    #[rstest]
    #[case(Metadata::test_fixture_metrics(), true)]
    #[case(Metadata::test_fixture_reboot(), false)]
    fn test_final_staging_path(#[case] metadata: Metadata, #[case] moved_to_persist: bool) {
        let tmp_dir = TempDir::new().unwrap();
        let tmp_path = tmp_dir.path();

        let tmp_staging_dir = tmp_path.join("tmp");
        let persist_staging_dir = tmp_path.join("persist");
        create_dir_all(&tmp_staging_dir).unwrap();
        create_dir_all(&persist_staging_dir).unwrap();

        let persist_config = PersistStorageConfig {
            logs: true,
            metrics: true,
            coredumps: true,
            reboots: false,
            min_headroom: 1024,
            max_usage: 1024 * 1024 * 1024,
            min_inodes: 1024,
        };

        let config = MarConfig::test_fixture_with_config(
            &tmp_staging_dir,
            &persist_staging_dir,
            persist_config,
        );

        let final_path = config.final_staging_path(&metadata);
        assert_eq!(
            final_path.starts_with(&persist_staging_dir),
            moved_to_persist
        );
    }

    impl MarConfig {
        pub fn test_fixture(tmp_dir: &Path, persist_dir: &Path) -> Self {
            Self {
                tmp_dir: tmp_dir.to_path_buf(),
                persist_dir: persist_dir.to_path_buf(),
                persist_config: None,
            }
        }

        pub fn test_fixture_with_config(
            tmp_dir: &Path,
            persist_dir: &Path,
            persist_config: PersistStorageConfig,
        ) -> Self {
            Self {
                tmp_dir: tmp_dir.to_path_buf(),
                persist_dir: persist_dir.to_path_buf(),
                persist_config: Some(persist_config),
            }
        }
    }
}
