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
use std::time::SystemTime;

use crate::config::CoredumpCaptureStrategy;
use crate::{build_info::VERSION, mar::LinuxLogsFormat};

use ciborium::{cbor, into_writer};
use eyre::Result;
use serde::Serialize;

use super::core_elf_note::build_elf_note;

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
        }
    }
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
        91,
        false
    )]
    #[case("threads", CoredumpCaptureStrategy::Threads{ max_thread_size: 32 * 1024}, 104, false)]
    #[case("app_logs", CoredumpCaptureStrategy::KernelSelection, 160, true)]
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
        };

        let map = serialize_metadata_as_map(&metadata).unwrap();
        let deser_map: Value = from_reader(map.as_slice()).unwrap();

        set_snapshot_suffix!("{}", test_name);
        insta::assert_debug_snapshot!(deser_map);
        assert_eq!(map.len(), expected_size);
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
