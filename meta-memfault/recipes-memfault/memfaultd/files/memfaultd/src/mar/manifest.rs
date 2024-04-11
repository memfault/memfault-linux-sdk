//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use eyre::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    build_info::VERSION,
    metrics::MetricStringKey,
    metrics::{MetricReportType, MetricValue},
    network::DeviceConfigRevision,
    network::NetworkConfig,
    reboot::RebootReason,
    util::serialization::{milliseconds_to_duration, optional_milliseconds_to_duration},
    util::system::{get_system_clock, read_system_boot_id, Clock},
};

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    schema_version: u32,
    pub collection_time: CollectionTime,
    device: Device,
    #[serde(default)]
    producer: Producer,
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

#[derive(Serialize, Deserialize)]
pub struct Producer {
    pub id: String,
    pub version: String,
}

impl Default for Producer {
    fn default() -> Self {
        Self {
            id: "memfaultd".into(),
            version: VERSION.to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone)]
pub enum CompressionAlgorithm {
    None,
    #[serde(rename = "zlib")]
    Zlib,
    #[serde(rename = "gzip")]
    Gzip,
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
        // PathBuf.file_name() -> OsString but serde does not handle it well
        // so we use a String here.
        log_file_name: String,
        #[serde(skip_serializing_if = "CompressionAlgorithm::is_none")]
        compression: CompressionAlgorithm,
        cid: Cid,
        next_cid: Cid,
    },
    #[serde(rename = "device-attributes")]
    DeviceAttributes { attributes: Vec<DeviceAttribute> },
    #[serde(rename = "device-config")]
    DeviceConfig { revision: DeviceConfigRevision },
    #[serde(rename = "elf-coredump")]
    ElfCoredump {
        coredump_file_name: String,
        #[serde(skip_serializing_if = "CompressionAlgorithm::is_none")]
        compression: CompressionAlgorithm,
    },
    #[serde(rename = "linux-reboot")]
    LinuxReboot { reason: RebootReason },
    // DEPRECATED but need to keep the variant for backwards compatibility
    // with MARs produced by earlier SDK versions
    #[serde(rename = "linux-heartbeat")]
    LinuxHeartbeat {
        #[serde(serialize_with = "crate::util::serialization::sorted_map::sorted_map")]
        metrics: HashMap<MetricStringKey, MetricValue>,
        #[serde(
            default,
            rename = "duration_ms",
            skip_serializing_if = "Option::is_none",
            with = "optional_milliseconds_to_duration"
        )]
        duration: Option<Duration>,
    },
    #[serde(rename = "linux-metric-report")]
    LinuxMetricReport {
        #[serde(serialize_with = "crate::util::serialization::sorted_map::sorted_map")]
        metrics: HashMap<MetricStringKey, MetricValue>,
        #[serde(rename = "duration_ms", with = "milliseconds_to_duration")]
        duration: Duration,
        report_type: MetricReportType,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinuxLogsFormat {
    id: String,
    serialization: String,
}

impl Default for LinuxLogsFormat {
    fn default() -> Self {
        Self {
            id: "v1".into(),
            serialization: "json-lines".into(),
        }
    }
}

// Note: Memfault manifest defines Cid as an object containing a Uuid.
#[derive(Serialize, Deserialize)]
pub struct Cid {
    uuid: Uuid,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct DeviceAttribute {
    string_key: MetricStringKey,
    value: Value,
}

impl DeviceAttribute {
    pub fn new(string_key: MetricStringKey, value: Value) -> Self {
        Self { string_key, value }
    }
}

impl<K: AsRef<str>, V: Into<Value>> TryFrom<(K, V)> for DeviceAttribute {
    type Error = String;

    fn try_from(value: (K, V)) -> std::result::Result<Self, Self::Error> {
        Ok(DeviceAttribute {
            string_key: str::parse(value.0.as_ref())?,
            value: value.1.into(),
        })
    }
}

impl Metadata {
    pub fn new_coredump(coredump_file_name: String, compression: CompressionAlgorithm) -> Self {
        Self::ElfCoredump {
            coredump_file_name,
            compression,
        }
    }

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
            format: LinuxLogsFormat::default(),
        }
    }

    pub fn new_device_attributes(attributes: Vec<DeviceAttribute>) -> Self {
        Self::DeviceAttributes { attributes }
    }

    pub fn new_device_config(revision: DeviceConfigRevision) -> Self {
        Self::DeviceConfig { revision }
    }

    pub fn new_reboot(reason: RebootReason) -> Self {
        Self::LinuxReboot { reason }
    }

    pub fn new_metric_report(
        metrics: HashMap<MetricStringKey, MetricValue>,
        duration: Duration,
        report_type: MetricReportType,
    ) -> Self {
        Self::LinuxMetricReport {
            metrics,
            duration,
            report_type,
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
            producer: Producer::default(),
            schema_version: 1,
            metadata,
        }
    }

    pub fn attachments(&self) -> Vec<String> {
        match &self.metadata {
            Metadata::ElfCoredump {
                coredump_file_name, ..
            } => vec![coredump_file_name.clone()],
            Metadata::LinuxLogs { log_file_name, .. } => vec![log_file_name.clone()],
            Metadata::DeviceAttributes { .. } => vec![],
            Metadata::DeviceConfig { .. } => vec![],
            Metadata::LinuxHeartbeat { .. } => vec![],
            Metadata::LinuxMetricReport { .. } => vec![],
            Metadata::LinuxReboot { .. } => vec![],
        }
    }
}

#[cfg(test)]
impl Metadata {
    pub fn test_fixture() -> Self {
        Metadata::new_device_config(0)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, str::FromStr};

    use crate::{
        mar::CompressionAlgorithm,
        metrics::{MetricReportType, MetricValue},
        reboot::RebootReasonCode,
    };
    use rstest::rstest;
    use uuid::uuid;

    use crate::network::NetworkConfig;
    use crate::reboot::RebootReason;

    use super::{CollectionTime, Manifest};

    #[rstest]
    #[case("coredump-gzip", CompressionAlgorithm::Gzip)]
    #[case("coredump-none", CompressionAlgorithm::None)]
    fn serialization_of_coredump(#[case] name: &str, #[case] compression: CompressionAlgorithm) {
        let config = NetworkConfig::test_fixture();

        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_coredump("/tmp/core.elf".into(), compression),
        );
        insta::assert_json_snapshot!(name, manifest, { ".producer.version" => "tests"});
    }

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
        insta::assert_json_snapshot!(name, manifest, { ".producer.version" => "tests" });
    }

    #[rstest]
    fn serialization_of_device_attributes() {
        let config = NetworkConfig::test_fixture();
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_device_attributes(vec![
                ("my_string", "foo").try_into().unwrap(),
                ("my_int", 123).try_into().unwrap(),
                ("my_float", 123.456).try_into().unwrap(),
                ("my_bool", true).try_into().unwrap(),
            ]),
        );
        insta::assert_json_snapshot!(manifest, { ".producer.version" => "tests"});
    }

    #[rstest]
    fn serialization_of_device_configc() {
        let config = NetworkConfig::test_fixture();
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_device_config(42),
        );
        insta::assert_json_snapshot!(manifest, { ".producer.version" => "tests"});
    }

    #[rstest]
    fn serialization_of_reboot() {
        let config = NetworkConfig::test_fixture();
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_reboot(RebootReason::from(RebootReasonCode::UserShutdown)),
        );
        insta::assert_json_snapshot!(manifest, { ".producer.version" => "tests"});
    }

    #[rstest]
    fn serialization_of_custom_reboot() {
        let config = NetworkConfig::test_fixture();
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_reboot(
                RebootReason::from_str("CustomRebootReason").unwrap_or_else(|e| panic!("{}", e)),
            ),
        );
        insta::assert_json_snapshot!(manifest, { ".producer.version" => "tests"});
    }

    #[rstest]
    fn serialization_of_custom_unexpected_reboot() {
        let config = NetworkConfig::test_fixture();
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::new_reboot(
                RebootReason::from_str("!CustomUnexpectedRebootReason")
                    .unwrap_or_else(|e| panic!("{}", e)),
            ),
        );
        insta::assert_json_snapshot!(manifest, { ".producer.version" => "tests"});
    }

    #[rstest]
    fn serialization_of_linux_heartbeat() {
        let config = NetworkConfig::test_fixture();
        let manifest = Manifest::new(
            &config,
            CollectionTime::test_fixture(),
            super::Metadata::LinuxMetricReport {
                metrics: HashMap::from([
                    ("n1".parse().unwrap(), MetricValue::Number(1.0)),
                    ("n2".parse().unwrap(), MetricValue::Number(42.0)),
                ]),
                duration: std::time::Duration::from_secs(42),
                report_type: MetricReportType::Heartbeat,
            },
        );
        insta::assert_json_snapshot!(manifest, { ".producer.version" => "tests"});
    }

    #[rstest]
    #[case("heartbeat")]
    #[case("heartbeat_with_duration")]
    #[case("metric_report")]
    #[case("device_config")]
    #[case("reboot")]
    #[case("attributes")]
    #[case("elf_coredump")]
    fn can_parse_test_manifests(#[case] name: &str) {
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/mar/test-manifests")
            .join(name)
            .with_extension("json");
        let manifest_json = std::fs::read_to_string(input_path).unwrap();
        let manifest: Manifest = serde_json::from_str(manifest_json.as_str()).unwrap();
        insta::assert_json_snapshot!(name, manifest, { ".producer.version" => "tests"});
    }
}
