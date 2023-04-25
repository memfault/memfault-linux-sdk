//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use std::time::Duration;
use std::{net::SocketAddr, path::PathBuf};

use crate::util::*;
use crate::util::{path::AbsolutePath, serialization::*};

#[derive(Serialize, Deserialize, Debug)]
pub struct MemfaultdConfig {
    #[serde(rename = "queue_size_kib", with = "kib_to_usize")]
    pub queue_size: usize,
    pub persist_dir: AbsolutePath,
    pub tmp_dir: Option<AbsolutePath>,
    #[serde(rename = "tmp_dir_min_headroom_kib", with = "kib_to_usize")]
    pub tmp_dir_min_headroom: usize,
    pub tmp_dir_min_inodes: usize,
    #[serde(rename = "tmp_dir_max_usage_kib", with = "kib_to_usize")]
    pub tmp_dir_max_usage: usize,
    #[serde(rename = "refresh_interval_seconds", with = "seconds_to_duration")]
    pub refresh_interval: Duration,
    pub enable_data_collection: bool,
    pub enable_dev_mode: bool,
    pub software_version: String,
    pub software_type: String,
    pub project_key: String,
    pub base_url: String,
    pub swupdate_plugin: SwUpdateConfig,
    pub reboot_plugin: RebootPlugin,
    pub collectd_plugin: CollectdPlugin,
    pub coredump_plugin: CoredumpPlugin,
    #[serde(rename = "fluent-bit")]
    pub fluent_bit: FluentBitConfig,
    pub logs: LogsConfig,
    pub mar: MarConfig,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SwUpdateConfig {
    pub input_file: PathBuf,
    pub output_file: PathBuf,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RebootPlugin {
    pub last_reboot_reason_file: PathBuf,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CollectdPlugin {
    pub header_include_output_file: PathBuf,
    pub footer_include_output_file: PathBuf,
    pub non_memfaultd_chain: String,
    #[serde(rename = "write_http_buffer_size_kib", with = "kib_to_usize")]
    pub write_http_buffer_size: usize,
    #[serde(rename = "interval_seconds", with = "seconds_to_duration")]
    pub interval: Duration,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum CoredumpCompression {
    #[serde(rename = "gzip")]
    Gzip,
    #[serde(rename = "none")]
    None,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CoredumpPlugin {
    pub compression: CoredumpCompression,
    #[serde(rename = "coredump_max_size_kib", with = "kib_to_usize")]
    pub coredump_max_size: usize,
    pub rate_limit_count: u32,
    #[serde(rename = "rate_limit_duration_seconds", with = "seconds_to_duration")]
    pub rate_limit_duration: Duration,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FluentBitConfig {
    pub extra_fluentd_attributes: Vec<String>,
    pub bind_address: SocketAddr,
    pub max_buffered_lines: usize,
    pub max_connections: usize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LogsConfig {
    #[serde(rename = "rotate_size_kib", with = "kib_to_usize")]
    pub rotate_size: usize,

    #[serde(rename = "rotate_after_seconds", with = "seconds_to_duration")]
    pub rotate_after: Duration,

    #[serde(with = "number_to_compression")]
    pub compression_level: Compression,

    pub max_lines_per_minute: NonZeroU32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MarConfig {
    #[serde(rename = "mar_file_max_size_kib", with = "kib_to_usize")]
    pub mar_file_max_size: usize,
}

use flate2::Compression;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct JsonConfigs {
    /// Built-in configuration and System configuration
    pub base: Value,
    /// Runtime configuration
    pub runtime: Value,
}

impl MemfaultdConfig {
    pub const DEFAULT_CONFIG_PATH: &'static str = "/etc/memfaultd.conf";

    pub fn load(config_path: Option<&Path>) -> eyre::Result<MemfaultdConfig> {
        let JsonConfigs {
            base: mut config,
            runtime,
        } = Self::parse_configs(config_path)?;
        Self::merge_into(&mut config, runtime);
        // Transform the JSON object into a typed structure.
        Ok(serde_json::from_value(config)?)
    }

    /// Parse config file from given path and returns (builtin+system config, runtime config).
    pub fn parse_configs(config_path: Option<&Path>) -> eyre::Result<JsonConfigs> {
        // Initialize with the builtin config file.
        let mut base: Value = Self::parse(include_str!("../../../libmemfaultc/builtin.conf"))?;

        // Select config file to read
        let user_config_path = config_path.unwrap_or_else(|| Path::new(Self::DEFAULT_CONFIG_PATH));

        // Read and parse the user config file.
        let user_config = Self::parse(std::fs::read_to_string(user_config_path)?.as_str())?;

        // Merge the two JSON objects together
        Self::merge_into(&mut base, user_config);

        // Load the runtime config but only if the file exists. (Missing runtime config is not an error.)
        let runtime_config_path = Self::runtime_config_path_from_json(&base)?;
        let runtime = if runtime_config_path.exists() {
            Self::parse(fs::read_to_string(runtime_config_path)?.as_str())?
        } else {
            Value::Object(serde_json::Map::new())
        };

        Ok(JsonConfigs { base, runtime })
    }

    /// Set and write boolean in runtime config.
    pub fn set_and_write_bool_to_runtime_config(&self, key: &str, value: bool) -> eyre::Result<()> {
        let config_string = match fs::read_to_string(self.runtime_config_path()) {
            Ok(config_string) => config_string,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "{}".to_string()
                } else {
                    return Err(eyre::eyre!("Failed to read runtime config: {}", e));
                }
            }
        };

        let mut config_val = Self::parse(&config_string)?;
        config_val[key] = Value::Bool(value);

        self.write_value_to_runtime_config(config_val)
    }

    /// Write config to runtime config file.
    ///
    /// This is used to write the config to a file that can be read by the memfaultd process.
    fn write_value_to_runtime_config(&self, value: Value) -> eyre::Result<()> {
        let runtime_config_path = self.runtime_config_path();
        fs::write(&runtime_config_path, value.to_string())?;

        Ok(())
    }

    pub fn runtime_config_path(&self) -> PathBuf {
        PathBuf::from(self.persist_dir.clone()).join("runtime.conf")
    }

    // Parse a Memfaultd JSON configuration file (with optional C-style comments) and return a serde_json::Value object.
    fn parse(config_string: &str) -> eyre::Result<Value> {
        let json_text = string::remove_comments(config_string);
        let json: Value = serde_json::from_str(json_text.as_str())?;
        if !json.is_object() {
            return Err(eyre::eyre!("Configuration should be a JSON object."));
        }
        Ok(json)
    }

    /// Merge two JSON objects together. The values from the second one will override values in the first one.
    fn merge_into(dest: &mut Value, src: Value) {
        assert!(dest.is_object() && src.is_object());
        if let Value::Object(dest_map) = src {
            for (key, value) in dest_map {
                if let Some(obj) = dest.get_mut(&key) {
                    if obj.is_object() {
                        MemfaultdConfig::merge_into(obj, value);
                        continue;
                    }
                }
                dest[&key] = value;
            }
        }
    }

    // Generate the path to the runtime config file from a serde_json::Value object. This should include the "persist_dir" field.
    fn runtime_config_path_from_json(config: &Value) -> eyre::Result<PathBuf> {
        let mut persist_dir = PathBuf::from(
            config["persist_dir"]
                .as_str()
                .ok_or(eyre::eyre!("Config['persist_dir'] must be a string."))?,
        );
        persist_dir.push("runtime.conf");
        Ok(persist_dir)
    }
}

#[cfg(test)]
impl MemfaultdConfig {
    pub fn test_fixture() -> Self {
        use std::fs::write;
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("memfaultd.conf");
        write(&config_path, "{}").unwrap();
        MemfaultdConfig::load(Some(&config_path)).unwrap()
    }
}

#[cfg(test)]
mod test {
    use rstest::rstest;

    use super::*;

    use crate::test_utils::set_snapshot_suffix;

    #[test]
    fn test_merge() {
        let mut c =
            serde_json::from_str(r##"{ "node": { "value": true, "valueB": false } }"##).unwrap();
        let j = serde_json::from_str(r##"{ "node2": "xxx" }"##).unwrap();

        MemfaultdConfig::merge_into(&mut c, j);

        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r##"{"node":{"value":true,"valueB":false},"node2":"xxx"}"##
        );
    }

    #[test]
    fn test_merge_overwrite() {
        let mut c =
            serde_json::from_str(r##"{ "node": { "value": true, "valueB": false } }"##).unwrap();
        let j = serde_json::from_str(r##"{ "node": { "value": false }}"##).unwrap();

        MemfaultdConfig::merge_into(&mut c, j);

        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r##"{"node":{"value":false,"valueB":false}}"##
        );
    }

    #[test]
    fn test_merge_overwrite_nested() {
        let mut c = serde_json::from_str(
            r##"{ "node": { "value": true, "valueB": false, "valueC": { "a": 1, "b": 2 } } }"##,
        )
        .unwrap();
        let j = serde_json::from_str(r##"{ "node": { "valueC": { "b": 42 } }}"##).unwrap();

        MemfaultdConfig::merge_into(&mut c, j);

        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r##"{"node":{"value":true,"valueB":false,"valueC":{"a":1,"b":42}}}"##
        );
    }

    #[rstest]
    #[case("empty_object")]
    #[case("with_partial_logs")]
    #[case("without_coredump_compression")]
    fn can_parse_test_files(#[case] name: &str) {
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/config/test-config")
            .join(name)
            .with_extension("json");
        // Verifies that the file is parsable
        let content = MemfaultdConfig::load(Some(&input_path)).unwrap();
        // And that the configuration generated is what we expect.
        // Use `cargo insta review` to quickly approve changes.
        insta::assert_json_snapshot!(name, content)
    }

    #[rstest]
    fn will_reject_invalid_tmp_path() {
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/config/test-config")
            .join("with_invalid_path")
            .with_extension("json");
        let result = MemfaultdConfig::load(Some(&input_path));
        dbg!("result: {:?}", &result);
        assert!(result.is_err());
    }

    #[rstest]
    #[case("no_file", None)]
    #[case("empty_object", Some("{}"))]
    #[case("other_key", Some(r##"{"key2":false}"##))]
    fn test_set_and_write_bool_to_runtime_config(
        #[case] test_name: &str,
        #[case] config_string: Option<&str>,
    ) {
        let mut config = MemfaultdConfig::test_fixture();
        let temp_data_dir = tempfile::tempdir().unwrap();
        config.persist_dir = AbsolutePath::try_from(temp_data_dir.path().to_path_buf()).unwrap();

        if let Some(config_string) = config_string {
            std::fs::write(config.runtime_config_path(), config_string).unwrap();
        }

        config
            .set_and_write_bool_to_runtime_config("key", true)
            .unwrap();

        let disk_config_string = std::fs::read_to_string(config.runtime_config_path()).unwrap();

        set_snapshot_suffix!("{}", test_name);
        insta::assert_json_snapshot!(disk_config_string);
    }
}
