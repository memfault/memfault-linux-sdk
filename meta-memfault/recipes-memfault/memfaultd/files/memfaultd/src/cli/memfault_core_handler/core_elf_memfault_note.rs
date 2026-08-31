//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Utilities and data types for writing Memfault-specific ELF notes to a core dump file.
//!
//! Currently we write two notes:
//!
//! 1. A note containing metadata about the core dump. This note is written by the
//!    `CoreHandler` whenever it receives a core dump. It contains information about the device,
//!    that will be used to associate the core dump with a device in the Memfault cloud.
//! 2. A note containing debug data about the core dump. Currently this note only contains
//!    logs written during the coredump capture process. These logs are used by Memfault to debug
//!    issues with coredump capture.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, ErrorKind};
use std::time::SystemTime;

use crate::config::{coredump_attribute_value_is_valid, CoredumpCaptureStrategy};
use crate::metrics::MetricStringKey;
use crate::util::path::AbsolutePath;
use crate::{build_info::VERSION, mar::LinuxLogsFormat};

use super::core_elf_note::build_elf_note;
use ciborium::{cbor, into_writer};
use eyre::{Result, WrapErr};
use log::warn;
use serde::Serialize;
use serde_json::Value;

const NOTE_NAME: &str = "Memfault\0";
const METADATA_NOTE_TYPE: u32 = 0x4154454d;
const DEBUG_DATA_NOTE_TYPE: u32 = 0x4154454e;
const MEMFAULT_CORE_ELF_METADATA_SCHEMA_VERSION_V1: u32 = 1;
const MEMFAULT_CORE_ELF_DEBUG_DATA_SCHEMA_VERSION_V1: u32 = 1;

/// Map of keys used in the Memfault core ELF metadata note.
///
/// Integer keys are used here instead of strings to reduce the size of the note.
enum MemfaultCoreElfMetadataKey {
    SchemaVersion = 1,
    LinuxSdkVersion = 2,
    CapturedTime = 3,
    DeviceSerial = 4,
    HardwareVersion = 5,
    SoftwareType = 6,
    SoftwareVersion = 7,
    CmdLine = 8,
    CaptureStrategy = 9,
    ApplicationLogs = 10,
    CustomAttributes = 11,
}

#[derive(Debug, Serialize)]
pub struct MemfaultMetadataLogs {
    logs: Vec<String>,
    format: LinuxLogsFormat,
}

impl MemfaultMetadataLogs {
    pub fn new(logs: Vec<String>, format: LinuxLogsFormat) -> Self {
        Self { logs, format }
    }
}

/// Metadata about a core dump.
#[derive(Debug)]
pub struct CoredumpMetadata {
    pub device_id: String,
    pub hardware_version: String,
    pub software_type: String,
    pub software_version: String,
    pub sdk_version: String,
    pub captured_time_epoch_s: u64,
    pub cmd_line: String,
    pub capture_strategy: CoredumpCaptureStrategy,
    pub app_logs: Option<MemfaultMetadataLogs>,
    pub custom_attributes: BTreeMap<String, Value>,
}

impl CoredumpMetadata {
    pub fn new(config: &crate::config::Config, cmd_line: String) -> Self {
        Self {
            device_id: config.device_info.device_id.clone(),
            hardware_version: config.device_info.hardware_version.clone(),
            software_type: config.software_type().to_string(),
            software_version: config.software_version().to_string(),
            sdk_version: VERSION.to_string(),
            captured_time_epoch_s: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            cmd_line,
            capture_strategy: config.config_file.coredump.capture_strategy,
            app_logs: None,
            custom_attributes: Self::build_custom_attributes(config),
        }
    }

    /// Collect the custom attributes to capture into the coredump.
    ///
    /// Any attributes configured under `coredump.attributes` are attached, and finally any
    /// attributes read from `coredump.attributes_file` are layered on top of those.
    ///
    /// The file is read fresh on every crash so device software can keep it up to
    /// date as live state changes without reloading memfaultd.
    fn build_custom_attributes(config: &crate::config::Config) -> BTreeMap<String, Value> {
        let mut attributes = BTreeMap::new();

        if let Some(configured) = &config.config_file.coredump.attributes {
            for attribute in configured {
                attributes.insert(attribute.key.as_str().to_string(), attribute.value.clone());
            }
        }

        if let Some(path) = &config.config_file.coredump.attributes_file {
            match read_attributes_file(path) {
                Ok(file_attributes) => attributes.extend(file_attributes),
                // A missing or malformed attributes file must never prevent capture, so we log and
                // continue with whatever attributes we already have.
                Err(e) => warn!("Ignoring coredump attributes file: {}", e),
            }
        }
        attributes
    }
}

/// Read a JSON key-value file and return its contents as a `BTreeMap<String, Value>`. Used in
/// coredumps to attach customer-set attributes that reflect live device state at crash time.
///
/// The file must contain a single JSON object mapping attribute names to values, e.g.
/// `{"developer_mode": true, "build_channel": "beta"}`. Keys are validated as `MetricStringKey`s
/// and values must be scalars (string, number, or boolean), matching the validation applied to
/// statically-configured `coredump.attributes`. Entries with invalid values are skipped
/// individually so one bad entry never discards the rest.
fn read_attributes_file(path: &AbsolutePath) -> Result<BTreeMap<String, Value>> {
    let file = match File::open(&**path) {
        Ok(file) => file,
        // The attributes file is optional: device software only writes it when it has live
        // attributes to report, and the configured default path is often absent. Treat a missing
        // file as "no attributes" rather than an error so we don't warn on every crash.
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => {
            return Err(e).wrap_err_with(|| {
                format!("Failed to open coredump attributes file {}", path.display())
            })
        }
    };
    let attributes: BTreeMap<String, Value> = serde_json::from_reader(BufReader::new(file))
        .wrap_err_with(|| {
            format!(
                "Failed to parse coredump attributes file {}",
                path.display()
            )
        })?;

    // Values must be scalars, same as statically-configured attributes. Skip invalid entries
    // individually so one bad value never discards the rest of the file.
    let mut out = BTreeMap::new();
    for (key_str, value) in attributes {
        let key: MetricStringKey = match key_str.parse() {
            Ok(key) => key,
            Err(e) => {
                warn!(
                    "Ignoring invalid coredump attribute key \"{}\" from {}: {}",
                    key_str,
                    path.display(),
                    e
                );
                continue;
            }
        };
        if let Err(e) = coredump_attribute_value_is_valid(&value) {
            warn!(
                "Ignoring invalid coredump attribute \"{}\" from {}: {}",
                key.as_str(),
                path.display(),
                e
            );
            continue;
        }
        out.insert(key.as_str().to_string(), value);
    }
    Ok(out)
}

/// Serialize a `CoredumpMetadata` struct as a CBOR map.
///
/// This CBOR map uses integer keys instead of strings to reduce the size of the note.
pub fn serialize_metadata_as_map(metadata: &CoredumpMetadata) -> Result<Vec<u8>> {
    let cbor_val = cbor!({
        MemfaultCoreElfMetadataKey::SchemaVersion as u32 => MEMFAULT_CORE_ELF_METADATA_SCHEMA_VERSION_V1,
        MemfaultCoreElfMetadataKey::LinuxSdkVersion as u32 => metadata.sdk_version,
        MemfaultCoreElfMetadataKey::CapturedTime as u32 => metadata.captured_time_epoch_s,
        MemfaultCoreElfMetadataKey::DeviceSerial as u32 => metadata.device_id,
        MemfaultCoreElfMetadataKey::HardwareVersion as u32 => metadata.hardware_version,
        MemfaultCoreElfMetadataKey::SoftwareType as u32 => metadata.software_type,
        MemfaultCoreElfMetadataKey::SoftwareVersion as u32 => metadata.software_version,
        MemfaultCoreElfMetadataKey::CmdLine as u32 => metadata.cmd_line,
        MemfaultCoreElfMetadataKey::CaptureStrategy as u32 => metadata.capture_strategy,
        MemfaultCoreElfMetadataKey::ApplicationLogs as u32 => metadata.app_logs,
        MemfaultCoreElfMetadataKey::CustomAttributes as u32 => metadata.custom_attributes,
    })?;

    let mut buffer = Vec::new();
    into_writer(&cbor_val, &mut buffer)?;

    Ok(buffer)
}

/// Write a core ELF note containing metadata about a core dump.
///
/// This note is written by the `CoreHandler` whenever it receives a core dump. It contains
/// information about the device, that will be used to associate the core dump with a device in the
/// Memfault cloud.
pub fn write_memfault_metadata_note(metadata: &CoredumpMetadata) -> Result<Vec<u8>> {
    let description_buffer = serialize_metadata_as_map(metadata)?;

    build_elf_note(NOTE_NAME, &description_buffer, METADATA_NOTE_TYPE)
}

/// A note containing a list of errors that occurred during coredump capture.
///
/// This note is written by the `CoreHandlerLogWrapper` when it receives an error or warning log.
/// These logs will help us debug issues with coredump capture.
#[derive(Debug, Serialize)]
pub struct CoredumpDebugData {
    pub schema_version: u32,
    pub capture_logs: Vec<String>,
}

/// Write a core ELF note containing debug data about the coredump capture process.
///
/// See `CoredumpDebugData` for more information.
pub fn write_memfault_debug_data_note(errors: Vec<String>) -> Result<Vec<u8>> {
    let coredump_capture_logs = CoredumpDebugData {
        schema_version: MEMFAULT_CORE_ELF_DEBUG_DATA_SCHEMA_VERSION_V1,
        capture_logs: errors,
    };

    let mut buffer = Vec::new();
    into_writer(&coredump_capture_logs, &mut buffer)?;

    build_elf_note(NOTE_NAME, &buffer, DEBUG_DATA_NOTE_TYPE)
}

#[cfg(test)]
mod test {
    use ciborium::{from_reader, Value};
    use rstest::rstest;

    use crate::test_utils::set_snapshot_suffix;

    use super::*;

    #[rstest]
    #[case(
        "kernel_selection",
        CoredumpCaptureStrategy::KernelSelection,
        93,
        false
    )]
    #[case("threads", CoredumpCaptureStrategy::Threads{ max_thread_size: 32 * 1024}, 106, false)]
    #[case("app_logs", CoredumpCaptureStrategy::KernelSelection, 162, true)]
    fn test_serialize_metadata_as_map(
        #[case] test_name: &str,
        #[case] capture_strategy: CoredumpCaptureStrategy,
        #[case] expected_size: usize,
        #[case] has_app_logs: bool,
    ) {
        let app_logs = has_app_logs.then(|| MemfaultMetadataLogs {
            logs: vec![
                "Error 1".to_string(),
                "Error 2".to_string(),
                "Error 3".to_string(),
            ],
            format: LinuxLogsFormat::default(),
        });
        let metadata = CoredumpMetadata {
            device_id: "12345678".to_string(),
            hardware_version: "evt".to_string(),
            software_type: "main".to_string(),
            software_version: "1.0.0".to_string(),
            sdk_version: "SDK_VERSION".to_string(),
            captured_time_epoch_s: 1234,
            cmd_line: "binary -a -b -c".to_string(),
            capture_strategy,
            app_logs,
            custom_attributes: BTreeMap::new(),
        };

        let map = serialize_metadata_as_map(&metadata).unwrap();
        let deser_map: Value = from_reader(map.as_slice()).unwrap();

        set_snapshot_suffix!("{}", test_name);
        insta::assert_debug_snapshot!(deser_map);
        assert_eq!(map.len(), expected_size);
    }

    #[test]
    fn test_serialize_metadata_with_custom_attributes() {
        let mut custom_attributes = BTreeMap::new();
        custom_attributes.insert("developer_mode".to_string(), serde_json::Value::Bool(true));
        custom_attributes.insert(
            "build_channel".to_string(),
            serde_json::Value::String("beta".to_string()),
        );
        let metadata = CoredumpMetadata {
            device_id: "12345678".to_string(),
            hardware_version: "evt".to_string(),
            software_type: "main".to_string(),
            software_version: "1.0.0".to_string(),
            sdk_version: "SDK_VERSION".to_string(),
            captured_time_epoch_s: 1234,
            cmd_line: "binary -a -b -c".to_string(),
            capture_strategy: CoredumpCaptureStrategy::KernelSelection,
            app_logs: None,
            custom_attributes,
        };

        let map = serialize_metadata_as_map(&metadata).unwrap();
        let deser_map: Value = from_reader(map.as_slice()).unwrap();

        insta::assert_debug_snapshot!(deser_map);
    }

    #[test]
    fn serialize_debug_data() {
        let capture_logs = CoredumpDebugData {
            schema_version: MEMFAULT_CORE_ELF_DEBUG_DATA_SCHEMA_VERSION_V1,
            capture_logs: vec![
                "Error 1".to_string(),
                "Error 2".to_string(),
                "Error 3".to_string(),
            ],
        };

        let mut capture_logs_buffer = Vec::new();
        into_writer(&capture_logs, &mut capture_logs_buffer).unwrap();

        let deser_capture_logs: Value = from_reader(capture_logs_buffer.as_slice()).unwrap();

        insta::assert_debug_snapshot!(deser_capture_logs);
    }
}

#[cfg(test)]
mod attributes_file_test {
    use std::fs::write;

    use rstest::rstest;
    use serde_json::{json, Value};
    use tempfile::{tempdir, TempDir};

    use crate::config::{Config, CoredumpAttribute};
    use crate::util::path::AbsolutePath;

    use super::{read_attributes_file, CoredumpMetadata};

    #[test]
    fn reads_valid_scalar_attributes() {
        let (_dir, path) = write_attributes_file(
            r#"{"developer_mode": true, "build_channel": "beta", "boot_count": 42}"#,
        );

        let attributes = read_attributes_file(&path).unwrap();

        assert_eq!(attributes.get("developer_mode"), Some(&json!(true)));
        assert_eq!(attributes.get("build_channel"), Some(&json!("beta")));
        assert_eq!(attributes.get("boot_count"), Some(&json!(42)));
        assert_eq!(attributes.len(), 3);
    }

    #[test]
    fn missing_file_is_empty_not_an_error() {
        let dir = tempdir().unwrap();
        let path = AbsolutePath::try_from(dir.path().join("does_not_exist.json")).unwrap();

        let attributes = read_attributes_file(&path).unwrap();

        assert!(attributes.is_empty());
    }

    #[test]
    fn malformed_json_is_an_error() {
        let (_dir, path) = write_attributes_file("not valid json");

        assert!(read_attributes_file(&path).is_err());
    }

    #[rstest]
    #[case::null(r#"{"good": "x", "bad": null}"#)]
    #[case::array(r#"{"good": "x", "bad": [1, 2]}"#)]
    #[case::object(r#"{"good": "x", "bad": {"nested": 1}}"#)]
    fn non_scalar_values_are_skipped_valid_ones_kept(#[case] contents: &str) {
        let (_dir, path) = write_attributes_file(contents);

        let attributes = read_attributes_file(&path).unwrap();

        assert_eq!(attributes.get("good"), Some(&json!("x")));
        assert!(!attributes.contains_key("bad"));
        assert_eq!(attributes.len(), 1);
    }

    #[test]
    fn file_values_override_configured_attributes_and_dev_mode() {
        let (_dir, path) = write_attributes_file(
            r#"{"developer_mode": false, "build_channel": "beta", "from_file": "x"}"#,
        );

        let mut config = Config::test_fixture();
        config.config_file.enable_dev_mode = true;
        config.config_file.coredump.attributes = Some(vec![
            CoredumpAttribute {
                key: "build_channel".into(),
                value: json!("stable"),
            },
            CoredumpAttribute {
                key: "boot_count".into(),
                value: json!(1),
            },
        ]);
        config.config_file.coredump.attributes_file = Some(path);

        let attributes = CoredumpMetadata::build_custom_attributes(&config);

        // developer_mode comes from the file only - and is never affected by
        // memfaultd's internal dev mode.
        assert_eq!(attributes.get("developer_mode"), Some(&json!(false)));
        // build_channel is configured as "stable" but the file overrides it to "beta".
        assert_eq!(attributes.get("build_channel"), Some(&json!("beta")));
        // boot_count only exists in the static config; it is preserved.
        assert_eq!(attributes.get("boot_count"), Some(&json!(1)));
        // from_file only exists in the file.
        assert_eq!(attributes.get("from_file"), Some(&json!("x")));
    }

    #[test]
    fn empty_object_yields_no_attributes() {
        let (_dir, path) = write_attributes_file("{}");

        let attributes = read_attributes_file(&path).unwrap();

        assert!(attributes.is_empty());
    }

    #[test]
    fn empty_file_is_an_error() {
        // A zero-byte file is distinct from a missing one: it is present but not valid JSON (no
        // top-level value), so it must surface as an error rather than "no attributes".
        let (_dir, path) = write_attributes_file("");

        assert!(read_attributes_file(&path).is_err());
    }

    #[rstest]
    #[case::array("[]")]
    #[case::array_of_objects(r#"[{"key": "value"}]"#)]
    #[case::string(r#""just a string""#)]
    #[case::number("42")]
    #[case::boolean("true")]
    #[case::null("null")]
    fn top_level_non_object_is_an_error(#[case] contents: &str) {
        // The file must be a JSON object mapping names to values. Any other top-level shape can't
        // deserialize into the attribute map and is rejected wholesale.
        let (_dir, path) = write_attributes_file(contents);

        assert!(read_attributes_file(&path).is_err());
    }

    #[rstest]
    #[case::empty_key(r#"{"good": "x", "": "y"}"#)]
    #[case::non_ascii_key(r#"{"good": "x", "tëmp": "y"}"#)]
    fn invalid_keys_are_skipped_valid_ones_kept(#[case] contents: &str) {
        // Keys are parsed into `MetricStringKey` individually, so an invalid key is dropped on its
        // own (with a warning) while the rest of the file is kept. This mirrors how invalid
        // *values* are handled, so one bad key never discards the whole file.
        let (_dir, path) = write_attributes_file(contents);

        let attributes = read_attributes_file(&path).unwrap();

        assert_eq!(attributes.get("good"), Some(&json!("x")));
        assert_eq!(attributes.len(), 1);
    }

    #[test]
    fn too_long_key_is_skipped() {
        // `MetricStringKey` caps keys at 128 characters; a 129-character key is invalid and is
        // skipped individually, leaving the valid entry intact.
        let long_key = "a".repeat(129);
        let (_dir, path) =
            write_attributes_file(&format!(r#"{{"kept": "x", "{}": "y"}}"#, long_key));

        let attributes = read_attributes_file(&path).unwrap();

        assert_eq!(attributes.get("kept"), Some(&json!("x")));
        assert_eq!(attributes.len(), 1);
    }

    #[test]
    fn all_non_scalar_values_yield_empty_map() {
        // Every value is invalid, so each is skipped individually and we are left with an empty
        // map rather than an error.
        let (_dir, path) = write_attributes_file(r#"{"a": null, "b": [1, 2], "c": {"n": 1}}"#);

        let attributes = read_attributes_file(&path).unwrap();

        assert!(attributes.is_empty());
    }

    #[test]
    fn duplicate_keys_keep_the_last_value() {
        // Keep only the last occurrence of a value.
        let (_dir, path) = write_attributes_file(r#"{"boot_count": 1, "boot_count": 2}"#);

        let attributes = read_attributes_file(&path).unwrap();

        assert_eq!(attributes.get("boot_count"), Some(&json!(2)));
        assert_eq!(attributes.len(), 1);
    }

    #[rstest]
    #[case::negative(r#"{"k": -7}"#, json!(-7))]
    #[case::zero(r#"{"k": 0}"#, json!(0))]
    #[case::float(r#"{"k": 2.5}"#, json!(2.5))]
    #[case::large_int(r#"{"k": 9007199254740993}"#, json!(9007199254740993_i64))]
    #[case::empty_string(r#"{"k": ""}"#, json!(""))]
    #[case::unicode_string(r#"{"k": "café ☕"}"#, json!("café ☕"))]
    fn scalar_value_varieties_are_preserved(#[case] contents: &str, #[case] expected: Value) {
        // Values are only checked for being scalar, not restricted to ASCII or a numeric range, so
        // the full breadth of JSON scalars must round-trip untouched.
        let (_dir, path) = write_attributes_file(contents);

        let attributes = read_attributes_file(&path).unwrap();

        assert_eq!(attributes.get("k"), Some(&expected));
    }

    #[test]
    fn directory_path_is_an_error() {
        // A path that resolves to a directory is neither missing nor valid JSON; it must error
        // rather than be mistaken for "no attributes".
        let dir = tempdir().unwrap();
        let path = AbsolutePath::try_from(dir.path().to_path_buf()).unwrap();

        assert!(read_attributes_file(&path).is_err());
    }

    #[test]
    fn build_custom_attributes_keeps_static_when_file_absent() {
        // A missing attributes file must not discard statically-configured attributes: capture
        // still proceeds with whatever the config provides.
        let dir = tempdir().unwrap();
        let missing = AbsolutePath::try_from(dir.path().join("absent.json")).unwrap();

        let mut config = Config::test_fixture();
        config.config_file.coredump.attributes = Some(vec![CoredumpAttribute {
            key: "build_channel".into(),
            value: json!("stable"),
        }]);
        config.config_file.coredump.attributes_file = Some(missing);

        let attributes = CoredumpMetadata::build_custom_attributes(&config);

        assert_eq!(attributes.get("build_channel"), Some(&json!("stable")));
        assert_eq!(attributes.len(), 1);
    }

    #[test]
    fn build_custom_attributes_is_empty_without_config_or_file() {
        // No static attributes and a missing file means no custom attributes at all.
        let dir = tempdir().unwrap();
        let missing = AbsolutePath::try_from(dir.path().join("absent.json")).unwrap();

        let mut config = Config::test_fixture();
        config.config_file.coredump.attributes = None;
        config.config_file.coredump.attributes_file = Some(missing);

        let attributes = CoredumpMetadata::build_custom_attributes(&config);

        assert!(attributes.is_empty());
    }

    /// Write `contents` to a file in a fresh temp dir and return both. The `TempDir` must be kept
    /// alive by the caller for the duration of the test or the file is deleted.
    fn write_attributes_file(contents: &str) -> (TempDir, AbsolutePath) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("coredump_attributes.json");
        write(&path, contents).unwrap();
        let path = AbsolutePath::try_from(path).unwrap();
        (dir, path)
    }
}
