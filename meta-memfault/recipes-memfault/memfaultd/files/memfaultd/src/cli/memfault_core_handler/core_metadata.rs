//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::time::SystemTime;

use crate::build_info::VERSION;

use ciborium::{cbor, into_writer};
use eyre::Result;

use super::core_elf_note::build_elf_note;

const NOTE_NAME: &str = "Memfault\0";
const NOTE_TYPE: u32 = 0x4154454d;
const MEMFAULT_CORE_ELF_METADATA_SCHEMA_VERSION_V1: u32 = 1;

enum MemfaultCoreElfMetadataKey {
    SchemaVersion = 1,
    LinuxSdkVersion = 2,
    CapturedTime = 3,
    DeviceSerial = 4,
    HardwareVersion = 5,
    SoftwareType = 6,
    SoftwareVersion = 7,
}

#[derive(Debug)]
pub struct CoredumpMetadata {
    pub device_id: String,
    pub hardware_version: String,
    pub software_type: String,
    pub software_version: String,
    pub sdk_version: String,
    pub captured_time_epoch_s: u64,
}

impl From<&crate::config::Config> for CoredumpMetadata {
    fn from(config: &crate::config::Config) -> Self {
        Self {
            device_id: config.device_info.device_id.clone(),
            hardware_version: config.device_info.hardware_version.clone(),
            software_type: config.config_file.software_type.clone(),
            software_version: config.config_file.software_version.clone(),
            sdk_version: VERSION.to_string(),
            captured_time_epoch_s: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

pub fn serialize_metadata_as_map(metadata: &CoredumpMetadata) -> Result<Vec<u8>> {
    let cbor_val = cbor!({
        MemfaultCoreElfMetadataKey::SchemaVersion as u32 => MEMFAULT_CORE_ELF_METADATA_SCHEMA_VERSION_V1,
        MemfaultCoreElfMetadataKey::LinuxSdkVersion as u32 => metadata.sdk_version,
        MemfaultCoreElfMetadataKey::CapturedTime as u32 => metadata.captured_time_epoch_s,
        MemfaultCoreElfMetadataKey::DeviceSerial as u32 => metadata.device_id,
        MemfaultCoreElfMetadataKey::HardwareVersion as u32 => metadata.hardware_version,
        MemfaultCoreElfMetadataKey::SoftwareType as u32 => metadata.software_type,
        MemfaultCoreElfMetadataKey::SoftwareVersion as u32 => metadata.software_version,
    })?;

    let mut buffer = Vec::new();
    into_writer(&cbor_val, &mut buffer)?;

    Ok(buffer)
}

pub fn write_memfault_note(metadata: &CoredumpMetadata) -> Result<Vec<u8>> {
    let description_buffer = serialize_metadata_as_map(metadata)?;

    build_elf_note(NOTE_NAME, &description_buffer, NOTE_TYPE)
}

#[cfg(test)]
mod test {
    use ciborium::{from_reader, Value};

    use super::*;

    #[test]
    fn test_serialize_metadata_as_map() {
        let metadata = CoredumpMetadata {
            device_id: "12345678".to_string(),
            hardware_version: "evt".to_string(),
            software_type: "main".to_string(),
            software_version: "1.0.0".to_string(),
            sdk_version: "SDK_VERSION".to_string(),
            captured_time_epoch_s: 1234,
        };

        let map = serialize_metadata_as_map(&metadata).unwrap();
        let deser_map: Value = from_reader(map.as_slice()).unwrap();
        insta::assert_debug_snapshot!(deser_map);
        assert_eq!(map.len(), 48);
    }
}
