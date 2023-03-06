//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{net::SocketAddr, path::PathBuf};

use crate::util::serialization::*;
use crate::util::*;

#[derive(Serialize, Deserialize, Debug)]
pub struct MemfaultdConfig {
    #[serde(rename = "queue_size_kib", with = "kib_to_usize")]
    pub queue_size: usize,
    pub data_dir: PathBuf,
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
    pub uboot_fw_env_file: PathBuf,
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
    #[serde(rename = "storage_min_headroom_kib", with = "kib_to_usize")]
    pub storage_min_headroom: usize,
    #[serde(rename = "storage_max_usage_kib", with = "kib_to_usize")]
    pub storage_max_usage: usize,
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

    pub tmp_folder: PathBuf,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MarConfig {
    #[serde(rename = "storage_max_usage_kib", with = "kib_to_usize")]
    pub storage_max_usage: usize,
}

use serde_json::Value;
use std::fs;
use std::path::Path;

impl MemfaultdConfig {
    const DEFAULT_CONFIG_PATH: &str = "/etc/memfaultd.conf";

    pub fn load(config_path: Option<&Path>) -> eyre::Result<MemfaultdConfig> {
        // Initialize with the builtin config file.
        let mut config: Value = Self::parse(include_str!("../../../libmemfaultc/builtin.conf"))?;

        // Select config file to read
        let user_config_path = config_path.unwrap_or_else(|| Path::new(Self::DEFAULT_CONFIG_PATH));

        // Read and parse the user config file.
        let user_config = Self::parse(std::fs::read_to_string(user_config_path)?.as_str())?;

        // Merge the two JSON objects together
        Self::merge_into(&mut config, user_config);

        // Load the runtime config but only if the file exists. (Missing runtime config is not an error.)
        let runtime_config_path = Self::generate_runtime_config_path(&config)?;
        if runtime_config_path.exists() {
            let runtime = Self::parse(fs::read_to_string(runtime_config_path)?.as_str())?;
            Self::merge_into(&mut config, runtime);
        }

        // Transform the JSON object into a typed structure.
        Ok(serde_json::from_value(config)?)
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

    // Generate the path to the runtime config file from a serde_json::Value object. This should include the "data_dir" field.
    fn generate_runtime_config_path(config: &Value) -> eyre::Result<PathBuf> {
        let mut data_dir = PathBuf::from(
            config["data_dir"]
                .as_str()
                .ok_or(eyre::eyre!("Config['data_dir'] must be a string."))?,
        );
        data_dir.push("runtime.conf");
        Ok(data_dir)
    }
}

#[cfg(test)]
mod test {
    use rstest::rstest;

    use super::*;

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
}
