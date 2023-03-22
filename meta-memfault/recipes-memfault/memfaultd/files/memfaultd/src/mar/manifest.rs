//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::time::Duration;

use chrono::Utc;
use eyre::Result;
use memfaultc_sys::build_info::memfaultd_sdk_version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    network::NetworkConfig,
    util::serialization::milliseconds_to_duration,
    util::system::{get_system_clock, read_system_boot_id, Clock},
};

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    schema_version: u32,
    pub collection_time: CollectionTime,
    device: Device,
    #[serde(flatten)]
    pub metadata: Metadata,
}

#[derive(Serialize, Deserialize)]
pub struct CollectionTime {
    pub timestamp: chrono::DateTime<Utc>,
    #[serde(rename = "uptime_ms", with = "milliseconds_to_duration")]
    uptime: Duration,
    linux_boot_id: Uuid,
    #[serde(rename = "elapsed_realtime_ms", with = "milliseconds_to_duration")]
    elapsed_realtime: Duration,
    // TODO: MFLT-9012 - remove these android only fields
    boot_count: u32,
}

#[derive(Serialize, Deserialize)]
struct Device {
    project_key: String,
    hardware_version: String,
    software_version: String,
    software_type: String,
    device_serial: String,
}

#[derive(Serialize, Deserialize, Copy, Clone)]
pub enum CompressionAlgorithm {
    None,
    #[serde(rename = "zlib")]
    Zlib,
}

impl CompressionAlgorithm {
    pub fn is_none(&self) -> bool {
        matches!(self, CompressionAlgorithm::None)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "metadata")]
pub enum Metadata {
    #[serde(rename = "linux-logs")]
    LinuxLogs {
        format: LinuxLogsFormat,
        producer: LinuxLogsProducer,
        // PathBuf.file_name() -> OsString but serde does not handle it well
        // so we use a String here.
        log_file_name: String,
        #[serde(skip_serializing_if = "CompressionAlgorithm::is_none")]
        compression: CompressionAlgorithm,
        cid: Cid,
        next_cid: Cid,
    },
}

#[derive(Serialize, Deserialize)]
pub struct LinuxLogsFormat {
    id: String,
    serialization: String,
}

#[derive(Serialize, Deserialize)]
pub struct LinuxLogsProducer {
    id: String,
    version: String,
}

// Note: Memfault manifest defines Cid as an object containing a Uuid.
#[derive(Serialize, Deserialize)]
pub struct Cid {
    uuid: Uuid,
}

impl Metadata {
    pub fn new_log(
        log_file_name: String,
        cid: Uuid,
        next_cid: Uuid,
        compression: CompressionAlgorithm,
    ) -> Self {
        Self::LinuxLogs {
            log_file_name,
            compression,
            cid: Cid { uuid: cid },
            next_cid: Cid { uuid: next_cid },
            format: LinuxLogsFormat {
                id: "v1".into(),
                serialization: "json-lines".into(),
            },
            producer: LinuxLogsProducer {
                id: "memfaultd".into(),
                version: memfaultd_sdk_version().to_owned(),
            },
        }
    }
}

impl CollectionTime {
    pub fn now() -> Result<Self> {
        Ok(Self {
            timestamp: Utc::now(),
            linux_boot_id: read_system_boot_id()?,
            uptime: get_system_clock(Clock::Monotonic)?,
            elapsed_realtime: get_system_clock(Clock::Boottime)?,
            // TODO: MFLT-9012 - remove these android only fields
            boot_count: 0,
        })
    }

    #[cfg(test)]
    pub fn test_fixture() -> Self {
        use chrono::TimeZone;
        use uuid::uuid;

        Self {
            timestamp: Utc.timestamp_millis_opt(1334250000000).unwrap(),
            uptime: Duration::new(10, 0),
            linux_boot_id: uuid!("413554b8-a727-11ed-b307-0317a0ffbea7"),
            elapsed_realtime: Duration::new(10, 0),
            // TODO: MFLT-9012 - remove these android only fields
            boot_count: 0,
        }
    }
}

impl From<&NetworkConfig> for Device {
    fn from(config: &NetworkConfig) -> Self {
        Self {
            project_key: config.project_key.clone(),
            device_serial: config.device_id.clone(),
            hardware_version: config.hardware_version.clone(),
            software_type: config.software_type.clone(),
            software_version: config.software_version.clone(),
        }
    }
}

impl Manifest {
    pub fn new(
        config: &NetworkConfig,
        collection_time: CollectionTime,
        metadata: Metadata,
    ) -> Self {
        Manifest {
            collection_time,
            device: Device::from(config),
            schema_version: 1,
            metadata,
        }
    }

    pub fn attachments(&self) -> impl Iterator<Item = String> {
        match &self.metadata {
            Metadata::LinuxLogs { log_file_name, .. } => std::iter::once(log_file_name.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::mar::manifest::CompressionAlgorithm;
    use rstest::rstest;
    use uuid::uuid;

    use crate::network::NetworkConfig;

    use super::{CollectionTime, Manifest};

    #[rstest]
    #[case("log-zlib", CompressionAlgorithm::Zlib)]
    #[case("log-none", CompressionAlgorithm::None)]
    fn serialization_of_log(#[case] name: &str, #[case] compression: CompressionAlgorithm) {
        let config = NetworkConfig::test_fixture();

        let this_cid = uuid!("99686390-a728-11ed-a68b-e7ff3cd0c7e7");
        let next_cid = uuid!("9e1ece10-a728-11ed-918e-5be35a10c7e7");
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_log("/var/log/syslog".into(), this_cid, next_cid, compression),
        );
        insta::assert_json_snapshot!(name, manifest, { ".metadata.producer.version" => "tests"});
    }
}
