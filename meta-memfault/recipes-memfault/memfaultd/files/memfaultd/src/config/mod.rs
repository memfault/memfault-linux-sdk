//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::eyre;
use itertools::Itertools;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::time::Duration;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use crate::{
    network::{NetworkClient, NetworkConfig},
    util::{DiskBacked, UnwrapOrDie, UpdateStatus},
};

use crate::util::disk_size::DiskSize;

#[cfg(test)]
pub use self::config_file::ConnectionCheckProtocol;

#[cfg(target_os = "linux")]
pub use self::config_file::{CoredumpCaptureStrategy, CoredumpCompression};

use self::device_info::DeviceInfoValue;
#[cfg(test)]
pub use self::device_info::MockDeviceInfoDefaults;
pub use self::{
    config_file::{
        ConnectivityMonitorConfig, ConnectivityMonitorTarget, JsonConfigs, LogSource,
        LogToMetricRule, MemfaultdConfig, SessionConfig, StorageConfig, SystemMetricConfig,
    },
    device_config::{DeviceConfig, Resolution, Sampling},
    device_info::{DeviceInfo, DeviceInfoDefaultsImpl, DeviceInfoWarning},
};

use crate::{
    mar::{MarEntryBuilder, Metadata},
    metrics::{DiskSpaceMetricsConfig, DiskstatsMetricsConfig, ProcessMetricsConfig},
};

use eyre::{Context, Result};

mod config_file;
pub use config_file::{LevelMappingConfig, LevelMappingRegex};

mod device_config;
mod device_info;
mod utils;

const FALLBACK_SOFTWARE_VERSION: &str = "0.0.0-memfault-unknown";
const FALLBACK_SOFTWARE_TYPE: &str = "memfault-unknown";

/// Container of the entire memfaultd configuration.
/// Implement `From<Config>` trait to initialize module specific configuration (see `NetworkConfig` for example).
pub struct Config {
    pub device_info: DeviceInfo,
    pub config_file: MemfaultdConfig,
    pub config_file_path: PathBuf,
    pub cached_device_config: Arc<RwLock<DiskBacked<DeviceConfig>>>,
}

const LOGS_SUBDIRECTORY: &str = "logs";
const MAR_STAGING_SUBDIRECTORY: &str = "mar";
const DEVICE_CONFIG_FILE: &str = "device_config.json";
const COREDUMP_RATE_LIMITER_FILENAME: &str = "coredump_rate_limit";

impl Config {
    pub const DEFAULT_CONFIG_PATH: &'static str = "/etc/memfaultd.conf";

    pub fn read_from_system(user_config: Option<&Path>) -> Result<Self> {
        // Select config file to read
        let config_file = user_config.unwrap_or_else(|| Path::new(Self::DEFAULT_CONFIG_PATH));

        let config = MemfaultdConfig::load(config_file).wrap_err(eyre!(
            "Unable to read config file {}",
            &config_file.display()
        ))?;

        let (device_info, warnings) =
            DeviceInfo::load().wrap_err(eyre!("Unable to load device info"))?;
        #[allow(clippy::print_stderr)]
        warnings.iter().for_each(|w| eprintln!("{}", w));

        let device_config = DiskBacked::from_path(&Self::device_config_path_from_config(&config));

        Ok(Self {
            device_info,
            config_file: config,
            config_file_path: config_file.to_owned(),
            cached_device_config: Arc::new(RwLock::new(device_config)),
        })
    }

    pub fn refresh_device_config(&self, client: &impl NetworkClient) -> Result<UpdateStatus> {
        let response = client.fetch_device_config()?;

        // Let the server know that we have applied the new version if it still
        // believes we have an older one.
        let confirm_version = match response.data.completed {
            Some(v) if v == response.data.revision => None,
            _ => Some(response.data.revision),
        };

        // Always write the config to our cache.
        let new_config: DeviceConfig = response.into();
        let update_status = self
            .cached_device_config
            .write()
            .unwrap_or_die()
            .set(new_config)?;

        // After saving, create the device-config confirmation MAR entry
        if let Some(revision) = confirm_version {
            let mar_staging = self.mar_staging_path();
            MarEntryBuilder::new(&mar_staging)?
                .set_metadata(Metadata::new_device_config(revision))
                .save(&NetworkConfig::from(self))?;
        }
        Ok(update_status)
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

    pub fn coredump_rate_limiter_file_path(&self) -> PathBuf {
        self.tmp_dir().join(COREDUMP_RATE_LIMITER_FILENAME)
    }

    pub fn logs_path(&self) -> PathBuf {
        self.tmp_dir().join(LOGS_SUBDIRECTORY)
    }

    pub fn mar_staging_path(&self) -> PathBuf {
        self.tmp_dir().join(MAR_STAGING_SUBDIRECTORY)
    }

    fn device_config_path_from_config(config_file: &MemfaultdConfig) -> PathBuf {
        config_file.persist_dir.join(DEVICE_CONFIG_FILE)
    }
    pub fn device_config_path(&self) -> PathBuf {
        Self::device_config_path_from_config(&self.config_file)
    }

    pub fn sampling(&self) -> Sampling {
        if self.config_file.enable_dev_mode {
            Sampling::development()
        } else {
            self.device_config().sampling
        }
    }

    /// Returns the device_config at the time of the call. If the device_config is updated
    /// after this call, the returned value will not be updated.
    /// This can block for a small moment if another thread is currently updating the device_config.
    fn device_config(&self) -> DeviceConfig {
        self.cached_device_config
            .read()
            // If another thread crashed while holding the mutex we want to crash the program
            .unwrap_or_die()
            // If we were not able to load from local-cache then return the defaults.
            .get()
            .clone()
    }

    /// Returns the software version for the device.
    ///
    /// The precedence is as follows:
    /// 1. Configured software version in device_info
    /// 2. Configured software version in config_file
    /// 3. Default software version in device_info
    /// 4. Fallback software version
    pub fn software_version(&self) -> &str {
        match (
            &self.device_info.software_version,
            &self.config_file.software_version,
        ) {
            (Some(DeviceInfoValue::Configured(sw_version)), _) => sw_version.as_ref(),
            (None, Some(sw_version)) => sw_version.as_ref(),
            (Some(DeviceInfoValue::Default(_)), Some(sw_version)) => sw_version.as_ref(),
            (Some(DeviceInfoValue::Default(sw_version)), None) => sw_version.as_ref(),
            (None, None) => FALLBACK_SOFTWARE_VERSION,
        }
    }

    /// Returns the software type for the device.
    ///
    /// The precedence is as follows:
    /// 1. Configured software type in device_info
    /// 2. Configured software type in config_file
    /// 3. Default software type in device_info
    /// 4. Fallback software type
    pub fn software_type(&self) -> &str {
        match (
            &self.device_info.software_type,
            &self.config_file.software_type,
        ) {
            (Some(DeviceInfoValue::Configured(software_type)), _) => software_type.as_ref(),
            (None, Some(software_type)) => software_type.as_ref(),
            (Some(DeviceInfoValue::Default(_)), Some(software_type)) => software_type.as_ref(),
            (Some(DeviceInfoValue::Default(software_type)), None) => software_type.as_ref(),
            (None, None) => FALLBACK_SOFTWARE_TYPE,
        }
    }

    pub fn mar_entry_max_age(&self) -> Duration {
        self.config_file.mar.mar_entry_max_age
    }

    pub fn mar_entry_max_count(&self) -> usize {
        self.config_file.mar.mar_entry_max_count
    }

    pub fn battery_monitor_periodic_update_enabled(&self) -> bool {
        self.config_file.battery_monitor.is_some()
    }

    pub fn battery_monitor_battery_info_command(&self) -> &str {
        match self.config_file.battery_monitor.as_ref() {
            Some(battery_config) => battery_config.battery_info_command.as_ref(),
            None => "",
        }
    }

    pub fn battery_monitor_interval(&self) -> Duration {
        match self.config_file.battery_monitor.as_ref() {
            Some(battery_config) => battery_config.interval_seconds,
            None => Duration::from_secs(0),
        }
    }

    pub fn connectivity_monitor_config(&self) -> Option<&ConnectivityMonitorConfig> {
        self.config_file.connectivity_monitor.as_ref()
    }

    pub fn session_configs(&self) -> Option<&Vec<SessionConfig>> {
        self.config_file.sessions.as_ref()
    }

    pub fn statsd_server_enabled(&self) -> bool {
        self.config_file.metrics.statsd_server.is_some()
    }

    pub fn statsd_server_address(&self) -> Result<SocketAddr> {
        match &self.config_file.metrics.statsd_server {
            Some(statsd_server_config) => Ok(statsd_server_config.bind_address),
            None => Err(eyre!("No StatsD server bind_address configured!")),
        }
    }

    // Returns true if Gauge metrics should be aggregated via
    // a running average rather than simply saving the most
    // recently reported value.
    //
    // Note: This config is only meant to be used for compatibility
    // with legacy implementations that sent Gauge StatsD metrics to
    // collectd which ultimately forwarded them to memfaultd where they
    // were averaged together. New implementations should use the Histogram
    // Metric type for Metrics where the desired aggregation is averaging.
    pub fn statsd_server_legacy_gauge_aggregation_enabled(&self) -> bool {
        match &self.config_file.metrics.statsd_server {
            // This config is irrelevant if the built-in StatsD server is enabled
            None => false,
            Some(statsd_server_config) => statsd_server_config
                .legacy_gauge_aggregation
                .unwrap_or(false),
        }
    }

    pub fn hrt_enabled(&self) -> bool {
        self.config_file.metrics.high_resolution_telemetry.enable
    }

    pub fn hrt_max_samples_per_min(&self) -> NonZeroU32 {
        self.config_file
            .metrics
            .high_resolution_telemetry
            .max_samples_per_minute
    }

    pub fn builtin_system_metric_collection_enabled(&self) -> bool {
        self.config_file.metrics.system_metric_collection.enable
    }

    pub fn system_metric_poll_interval(&self) -> Duration {
        self.config_file
            .metrics
            .system_metric_collection
            .poll_interval_seconds
    }

    pub fn system_metric_monitored_processes(&self) -> ProcessMetricsConfig {
        match self
            .config_file
            .metrics
            .system_metric_collection
            .processes
            .as_ref()
        {
            Some(processes) => {
                let mut processes = processes.clone();
                processes.insert("memfaultd".to_string());
                ProcessMetricsConfig::Processes(processes)
            }
            None => ProcessMetricsConfig::Auto,
        }
    }

    pub fn system_metric_disk_space_config(&self) -> DiskSpaceMetricsConfig {
        match self
            .config_file
            .metrics
            .system_metric_collection
            .disk_space
            .as_ref()
        {
            Some(mounts) => DiskSpaceMetricsConfig::Disks(mounts.clone()),
            None => DiskSpaceMetricsConfig::Auto,
        }
    }

    pub fn system_metric_diskstats_config(&self) -> DiskstatsMetricsConfig {
        match self
            .config_file
            .metrics
            .system_metric_collection
            .diskstats
            .as_ref()
        {
            Some(devices) => DiskstatsMetricsConfig::Devices(devices.clone()),
            None => DiskstatsMetricsConfig::Auto,
        }
    }

    pub fn system_metric_network_interfaces_config(&self) -> Option<&HashSet<String>> {
        self.config_file
            .metrics
            .system_metric_collection
            .network_interfaces
            .as_ref()
    }

    pub fn system_metric_config(&self) -> SystemMetricConfig {
        self.config_file.metrics.system_metric_collection.clone()
    }

    pub fn log_extraction_config(&self) -> &LevelMappingConfig {
        &self.config_file.logs.level_mapping
    }

    /// Combines both fluentd_extra_attributes and logs.extra_attributes
    ///
    /// All duplicates will be removed.
    pub fn log_extra_attributes(&self) -> Vec<String> {
        let fluentd_extra_attr = &self.config_file.fluent_bit.extra_fluentd_attributes;
        let logs_extra_attr = &self.config_file.logs.extra_attributes;

        fluentd_extra_attr
            .iter()
            .chain(logs_extra_attr.iter())
            .dedup()
            .cloned()
            .collect()
    }

    /// Gets the max buffered lines configured.
    ///
    /// This prefers any user configured value for fluent-bit to maintain
    /// backwards compatibility.
    pub fn log_max_buffered_lines(&self) -> usize {
        let fluent_bit_max = self.config_file.fluent_bit.max_buffered_lines;
        let log_max = self.config_file.logs.max_buffered_lines;

        fluent_bit_max.unwrap_or(log_max)
    }
}

#[cfg(test)]
impl Config {
    pub fn test_fixture() -> Self {
        Config {
            device_info: DeviceInfo::test_fixture(),
            config_file: MemfaultdConfig::test_fixture(),
            config_file_path: PathBuf::from("test_fixture.conf"),
            cached_device_config: Arc::new(RwLock::new(DiskBacked::from_path(&PathBuf::from(
                "/dev/null",
            )))),
        }
    }

    pub fn test_fixture_with_info_overrides(software_version: &str, software_type: &str) -> Self {
        Config {
            device_info: DeviceInfo::test_fixture_with_overrides(software_version, software_type),
            config_file: MemfaultdConfig::test_fixture(),
            config_file_path: PathBuf::from("test_fixture.conf"),
            cached_device_config: Arc::new(RwLock::new(DiskBacked::from_path(&PathBuf::from(
                "/dev/null",
            )))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{fs::create_dir_all, path::PathBuf};

    use rstest::{fixture, rstest};

    use crate::{
        config::{device_info::DeviceInfoValue, Config},
        mar::MarEntry,
        network::{
            DeviceConfigResponse, DeviceConfigResponseConfig, DeviceConfigResponseData,
            DeviceConfigResponseResolution, MockNetworkClient,
        },
        util::path::AbsolutePath,
    };

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

    #[test]
    fn test_info_overrides_file() {
        let config =
            Config::test_fixture_with_info_overrides("1.0.0-overridden", "overridden-type");

        assert_eq!(config.software_version(), "1.0.0-overridden");
        assert_eq!(config.software_type(), "overridden-type");
    }

    #[rstest]
    fn generate_mar_device_config_confirmation_when_needed(mut fixture: Fixture) {
        fixture
            .client
            .expect_fetch_device_config()
            .return_once(|| Ok(DEVICE_CONFIG_SAMPLE));
        fixture
            .config
            .refresh_device_config(&fixture.client)
            .unwrap();

        assert_eq!(fixture.count_mar_entries(), 1);
    }

    #[rstest]
    fn do_not_generate_mar_device_config_if_not_needed(mut fixture: Fixture) {
        let mut device_config = DEVICE_CONFIG_SAMPLE;
        device_config.data.completed = Some(device_config.data.revision);

        fixture
            .client
            .expect_fetch_device_config()
            .return_once(move || Ok(device_config));
        fixture
            .config
            .refresh_device_config(&fixture.client)
            .unwrap();

        assert_eq!(fixture.count_mar_entries(), 0);
    }

    #[rstest]
    #[case(Some(DeviceInfoValue::Configured("1.0.0".into())), None, "1.0.0")]
    #[case(Some(DeviceInfoValue::Default("1.0.0".into())), None, "1.0.0")]
    #[case(Some(DeviceInfoValue::Configured("1.0.0".into())), Some("2.0.0"), "1.0.0")]
    #[case(Some(DeviceInfoValue::Default("1.0.0".into())), Some("2.0.0"), "2.0.0")]
    #[case(None, Some("2.0.0"), "2.0.0")]
    #[case(None, None, FALLBACK_SOFTWARE_VERSION)]
    fn software_version_precedence(
        #[case] device_info_swv: Option<DeviceInfoValue>,
        #[case] config_swv: Option<&str>,
        #[case] expected: &str,
    ) {
        let mut config = Config::test_fixture();
        config.device_info.software_version = device_info_swv;
        config.config_file.software_version = config_swv.map(String::from);

        assert_eq!(config.software_version(), expected);
    }

    #[rstest]
    #[case(Some(DeviceInfoValue::Configured("test".into())), None, "test")]
    #[case(Some(DeviceInfoValue::Default("test".into())), None, "test")]
    #[case(Some(DeviceInfoValue::Configured("test".into())), Some("prod"), "test")]
    #[case(Some(DeviceInfoValue::Default("test".into())), Some("prod"), "prod")]
    #[case(None, Some("prod"), "prod")]
    #[case(None, None, FALLBACK_SOFTWARE_TYPE)]
    fn software_type_precedence(
        #[case] device_info_swv: Option<DeviceInfoValue>,
        #[case] config_swv: Option<&str>,
        #[case] expected: &str,
    ) {
        let mut config = Config::test_fixture();
        config.device_info.software_type = device_info_swv;
        config.config_file.software_type = config_swv.map(String::from);

        assert_eq!(config.software_type(), expected);
    }

    #[rstest]
    #[case::both_set(
        vec!["fluentd"],
        vec!["logs"],
        vec!["fluentd", "logs"])]
    #[case::fluentd_set(vec!["fluentd"], vec![], vec!["fluentd"])]
    #[case::logs_set(vec![], vec!["logs"], vec!["logs"])]
    #[case::neither_set(vec![], vec![], vec![])]
    #[case::duplicates(
        vec!["fluentd", "logs"],
        vec!["logs"],
        vec!["fluentd", "logs"])]
    fn log_attr_merge(
        #[case] fluentd_attr: Vec<&str>,
        #[case] log_attr: Vec<&str>,
        #[case] expected_attr: Vec<&str>,
    ) {
        let mut config = Config::test_fixture();
        config.config_file.fluent_bit.extra_fluentd_attributes =
            fluentd_attr.into_iter().map(|s| s.to_string()).collect();
        config.config_file.logs.extra_attributes =
            log_attr.into_iter().map(|s| s.to_string()).collect();

        let mut actual_attr = config.log_extra_attributes();
        actual_attr.sort();
        assert_eq!(actual_attr, expected_attr);
    }

    #[rstest]
    #[case(Some(42), 25, 42)]
    #[case(None, 25, 25)]
    fn log_max_buffered_precedence(
        #[case] fluent_bit_max: Option<usize>,
        #[case] log_max: usize,
        #[case] expected: usize,
    ) {
        let mut config = Config::test_fixture();
        config.config_file.fluent_bit.max_buffered_lines = fluent_bit_max;
        config.config_file.logs.max_buffered_lines = log_max;

        let actual_max = config.log_max_buffered_lines();
        assert_eq!(actual_max, expected);
    }

    struct Fixture {
        config: Config,
        _tmp_dir: tempfile::TempDir,
        client: MockNetworkClient,
    }

    #[fixture]
    fn fixture() -> Fixture {
        Fixture::new()
    }

    impl Fixture {
        fn new() -> Self {
            let tmp_dir = tempfile::tempdir().unwrap();
            let mut config = Config::test_fixture();
            config.config_file.persist_dir = tmp_dir.path().to_path_buf().try_into().unwrap();
            create_dir_all(config.mar_staging_path()).unwrap();
            Self {
                config,
                _tmp_dir: tmp_dir,
                client: MockNetworkClient::new(),
            }
        }

        fn count_mar_entries(self) -> usize {
            MarEntry::iterate_from_container(&self.config.mar_staging_path())
                .unwrap()
                .count()
        }
    }

    const DEVICE_CONFIG_SAMPLE: DeviceConfigResponse = DeviceConfigResponse {
        data: DeviceConfigResponseData {
            completed: None,
            revision: 42,
            config: DeviceConfigResponseConfig {
                memfault: crate::network::DeviceConfigResponseMemfault {
                    sampling: crate::network::DeviceConfigResponseSampling {
                        debugging_resolution: DeviceConfigResponseResolution::High,
                        logging_resolution: DeviceConfigResponseResolution::High,
                        monitoring_resolution: DeviceConfigResponseResolution::High,
                    },
                },
            },
        },
    };
}
