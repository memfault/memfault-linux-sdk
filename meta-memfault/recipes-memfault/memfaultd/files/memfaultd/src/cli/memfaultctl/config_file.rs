//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Cli commands for modifying the Memfaultd config file.

use crate::config::Config;
use crate::service_manager::{MemfaultdService, MemfaultdServiceManager};
use crate::util::string::capitalize;

use eyre::Result;
use urlencoding::encode;

/// Set the developer mode flag in the config file and restart memfaultd.
pub fn set_developer_mode(
    config: &mut Config,
    service_manager: &impl MemfaultdServiceManager,
    enable_dev_mode: bool,
) -> Result<()> {
    let already_set = check_already_set(
        "developer mode",
        config.config_file.enable_dev_mode,
        enable_dev_mode,
    );

    print_server_side_developer_mode_url(config);

    if already_set {
        return Ok(());
    }

    config.config_file.enable_dev_mode = enable_dev_mode;
    write_bool_to_config_and_restart_memfaultd(
        config,
        "enable_dev_mode",
        enable_dev_mode,
        service_manager,
    )
}

fn print_server_side_developer_mode_url(config: &Config) {
    let device_serial = encode(&config.device_info.device_id);
    let project_key = encode(&config.config_file.project_key);
    println!(
        "⚠️ Enable 'server-side developer mode' to bypass rate limits in Memfault cloud:\n\
        https://mflt.io/developer-mode?d={device_serial}&p={project_key}"
    );
}

/// Set the data collection flag in the config file and restart memfaultd.
pub fn set_data_collection(
    config: &mut Config,
    service_manager: &impl MemfaultdServiceManager,
    enable_data_collection: bool,
) -> Result<()> {
    if check_already_set(
        "data collection",
        config.config_file.enable_data_collection,
        enable_data_collection,
    ) {
        return Ok(());
    }

    config.config_file.enable_data_collection = enable_data_collection;
    write_bool_to_config_and_restart_memfaultd(
        config,
        "enable_data_collection",
        enable_data_collection,
        service_manager,
    )
}

fn check_already_set(module: &str, config_val: bool, new_value: bool) -> bool {
    let is_set = config_val == new_value;
    if is_set {
        let enable_string = if new_value { "enabled" } else { "disabled" };
        println!("{} is already {}", capitalize(module), enable_string);
    } else {
        let enable_string = if new_value { "Enabling" } else { "Disabling" };
        println!("{} {}", enable_string, module);
    }

    is_set
}

fn write_bool_to_config_and_restart_memfaultd(
    config: &Config,
    key: &str,
    value: bool,
    service_manager: &impl MemfaultdServiceManager,
) -> Result<()> {
    config
        .config_file
        .set_and_write_bool_to_runtime_config(key, value)?;

    service_manager.restart_service_if_running(MemfaultdService::Memfaultd)
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{service_manager::MockMemfaultdServiceManager, util::path::AbsolutePath};

    use rstest::rstest;
    use tempfile::tempdir;

    struct TestContext {
        config: Config,
        mock_service_manager: MockMemfaultdServiceManager,
        _tmpdir: tempfile::TempDir,
    }

    impl TestContext {
        fn new() -> Self {
            let mut config = Config::test_fixture();
            let mut mock_service_manager = MockMemfaultdServiceManager::new();

            let tmpdir = tempdir().unwrap();
            config.config_file.persist_dir =
                AbsolutePath::try_from(tmpdir.path().to_path_buf()).unwrap();

            mock_service_manager
                .expect_restart_service_if_running()
                .returning(|_| Ok(()));

            Self {
                config,
                mock_service_manager,
                _tmpdir: tmpdir,
            }
        }
    }

    #[rstest]
    #[case(true, false)]
    #[case(false, true)]
    #[case(false, false)]
    #[case(true, true)]
    fn test_set_developer_mode(#[case] enable_developer_mode: bool, #[case] initial_state: bool) {
        let mut test_context = TestContext::new();
        test_context.config.config_file.enable_dev_mode = initial_state;

        set_developer_mode(
            &mut test_context.config,
            &test_context.mock_service_manager,
            enable_developer_mode,
        )
        .unwrap();

        assert_eq!(
            test_context.config.config_file.enable_dev_mode,
            enable_developer_mode,
        );
    }

    #[rstest]
    #[case(true, false)]
    #[case(false, true)]
    #[case(false, false)]
    #[case(true, true)]
    fn test_set_data_collection(#[case] enable_data_collection: bool, #[case] initial_state: bool) {
        let mut test_context = TestContext::new();
        test_context.config.config_file.enable_data_collection = initial_state;

        set_data_collection(
            &mut test_context.config,
            &test_context.mock_service_manager,
            enable_data_collection,
        )
        .unwrap();

        assert_eq!(
            test_context.config.config_file.enable_data_collection,
            enable_data_collection,
        );
    }
}
