//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use super::NetworkConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{LogFilterRule, LogRuleAction},
    util::serialization::optional_datetime_to_rfc3339,
};

/// Device metadata required to prepare and commit uploads.
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadDeviceMetadata<'a> {
    device_serial: &'a str,
    hardware_version: &'a str,
    software_version: &'a str,
    software_type: &'a str,
}

impl<'a> From<&'a NetworkConfig> for UploadDeviceMetadata<'a> {
    fn from(config: &'a NetworkConfig) -> Self {
        UploadDeviceMetadata {
            device_serial: config.device_id.as_str(),
            hardware_version: config.hardware_version.as_str(),
            software_type: config.software_type.as_str(),
            software_version: config.software_version.as_str(),
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UploadPrepareKind {
    Mar,
}

/// Request body to prepare an upload
#[derive(Serialize, Debug)]
pub struct UploadPrepareRequest<'a> {
    content_encoding: Option<&'static str>,
    size: usize,
    device: UploadDeviceMetadata<'a>,
    kind: UploadPrepareKind,
}

impl<'a> UploadPrepareRequest<'a> {
    pub fn prepare(
        config: &'a NetworkConfig,
        filesize: usize,
        gzipped: bool,
        kind: UploadPrepareKind,
    ) -> UploadPrepareRequest<'a> {
        UploadPrepareRequest {
            content_encoding: if gzipped { Some("gzip") } else { None },
            size: filesize,
            device: UploadDeviceMetadata::from(config),
            kind,
        }
    }
}

/// Response for prepare-upload request
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadPrepareResponse {
    pub data: UploadPrepareResponseData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UploadPrepareResponseData {
    pub upload_url: String,
    pub token: String,
}

#[derive(Serialize, Debug)]
pub struct PreparedFile<'a> {
    token: &'a str,
}

#[derive(Serialize, Debug)]
pub struct MarUploadMetadata<'a> {
    device_serial: &'a str,
    file: PreparedFile<'a>,
    hardware_version: &'a str,
    software_type: &'a str,
    software_version: &'a str,
}

impl<'a> MarUploadMetadata<'a> {
    pub fn prepare(config: &'a NetworkConfig, token: &'a str) -> Self {
        Self {
            device_serial: &config.device_id,
            hardware_version: &config.hardware_version,
            software_type: &config.software_type,
            software_version: &config.software_version,
            file: PreparedFile { token },
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct DeviceConfigDeviceInfo<'a> {
    device_serial: &'a str,
    hardware_version: &'a str,
    software_version: &'a str,
    software_type: &'a str,
}

/// Device metadata required to prepare and commit uploads.
#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigRequest<'a> {
    #[serde(borrow)]
    device: DeviceConfigDeviceInfo<'a>,
}

impl<'a> From<&'a NetworkConfig> for DeviceConfigRequest<'a> {
    fn from(config: &'a NetworkConfig) -> Self {
        DeviceConfigRequest {
            device: DeviceConfigDeviceInfo {
                device_serial: config.device_id.as_str(),
                hardware_version: config.hardware_version.as_str(),
                software_type: config.software_type.as_str(),
                software_version: config.software_version.as_str(),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponse {
    pub data: DeviceConfigResponseData,
}

pub type DeviceConfigRevision = u32;

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponseData {
    pub config: DeviceConfigResponseConfig,
    pub revision: DeviceConfigRevision,
    pub completed: Option<DeviceConfigRevision>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponseConfig {
    pub memfault: DeviceConfigResponseMemfault,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponseMemfault {
    pub sampling: DeviceConfigResponseSampling,
    #[serde(with = "optional_datetime_to_rfc3339", default)]
    pub data_upload_start_date: Option<DateTime<Utc>>,
    pub memfaultd: Option<DeviceConfigResponseMemfaultd>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponseMemfaultd {
    #[serde(rename = "sdk-settings")]
    pub sdk_settings: Option<DeviceConfigSdkSettings>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigSdkSettings {
    #[serde(rename = "logs.filters")]
    pub log_filters: Option<DeviceConfigResponseLogFilters>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponseLogFilters {
    pub default_action: Option<LogRuleAction>,
    #[serde(rename = "rules")]
    pub log_filter_rules: Option<Vec<LogFilterRule>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeviceConfigResponseSampling {
    #[serde(rename = "debugging.resolution")]
    pub debugging_resolution: DeviceConfigResponseResolution,
    #[serde(rename = "logging.resolution")]
    pub logging_resolution: DeviceConfigResponseResolution,
    #[serde(rename = "monitoring.resolution")]
    pub monitoring_resolution: DeviceConfigResponseResolution,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DeviceConfigResponseResolution {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "high")]
    High,
}

#[cfg(test)]
mod test {
    use super::*;

    use insta::assert_json_snapshot;
    use serde_json::{json, Value};

    #[test]
    fn test_prepare_upload_serialization() {
        let network_config = NetworkConfig {
            project_key: "project_key".to_string(),
            base_url: "base_url".to_string(),
            device_id: "device_id".to_string(),
            hardware_version: "hardware_version".to_string(),
            software_version: "software_version".to_string(),
            software_type: "software_type".to_string(),
        };
        let prepare_request =
            UploadPrepareRequest::prepare(&network_config, 123, false, UploadPrepareKind::Mar);

        assert_json_snapshot!(prepare_request);
    }

    #[test]
    fn test_device_config() {
        let json = json!({
            "data": {
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "normal",
                            "logging.resolution": "high",
                            "monitoring.resolution": "low"
                        },
                        "data_upload_start_date": "1991-03-25T00:00:00Z"
                    }
                },
                "revision": 123,
                "completed": 100
            }
        });

        let json_string = json.to_string();
        let parsed: Result<DeviceConfigResponse, _> = serde_json::from_str(&json_string);

        assert!(parsed.is_ok());

        let response = parsed.unwrap();

        assert_json_snapshot!(response);
    }

    #[test]
    fn test_device_config_missing_upload_start() {
        let json = json!({
            "data": {
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "normal",
                            "logging.resolution": "high",
                            "monitoring.resolution": "low"
                        }
                    },
                },
                "revision": 123,
                "completed": 100
            }
        });

        let json_string = json.to_string();
        let parsed: Result<DeviceConfigResponse, _> = serde_json::from_str(&json_string);

        assert!(parsed.is_ok());

        let response = parsed.unwrap();

        assert_json_snapshot!(response);
    }

    #[test]
    fn test_unknown_device_config_field_deserialize() {
        let json_with_unknown = json!({
            "data": {
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "normal",
                            "logging.resolution": "high",
                            "monitoring.resolution": "low",
                            "unknown_sampling_field": "some_value"
                        },
                        "unknown_memfault_field": true
                    },
                    "unknown_config_field": 42
                },
                "revision": 123,
                "completed": 100,
                "unknown_data_field": "hello"
            },
            "unknown_root_field": ["array", "values"]
        });

        let json_string = json_with_unknown.to_string();
        let parsed: Result<DeviceConfigResponse, _> = serde_json::from_str(&json_string);

        assert!(parsed.is_ok());

        let response = parsed.unwrap();

        assert_eq!(response.data.revision, 123);
        assert_eq!(response.data.completed, Some(100));
        assert!(matches!(
            response.data.config.memfault.sampling.debugging_resolution,
            DeviceConfigResponseResolution::Normal
        ));
        assert!(matches!(
            response.data.config.memfault.sampling.logging_resolution,
            DeviceConfigResponseResolution::High
        ));
        assert!(matches!(
            response.data.config.memfault.sampling.monitoring_resolution,
            DeviceConfigResponseResolution::Low
        ));
    }

    #[test]
    fn test_device_config_with_memfaultd() {
        let json = json!({
            "data": {
                "revision": 42,
                "completed": 50,
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "high",
                            "logging.resolution": "normal",
                            "monitoring.resolution": "low"
                        },
                        "memfaultd": {
                            "sdk-settings": {
                                "logs.filters": {
                                    "default_action": null,
                                    "rules": [
                                        {
                                            "service": "example.service",
                                            "counter_name": null,
                                            "pattern": ".*error.*",
                                            "level": "ERROR",
                                            "extra_fields": {
                                                "tag": "important"
                                            },
                                            "action": "include"
                                        },
                                        {
                                            "service": null,
                                            "counter_name": null,
                                            "pattern": "debug",
                                            "level": null,
                                            "extra_fields": null,
                                            "action": "exclude"
                                        }
                                    ]
                                }
                            }
                        }
                    }
                },
                "data_upload_start_date": "2025-01-15T10:30:00Z"
            }
        });

        verify_response("device_config_memfaultd", json, 42, Some(50));
    }

    #[test]
    fn test_device_config_with_memfaultd_empty_memfaultd() {
        let json = json!({
            "data": {
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "high",
                            "logging.resolution": "normal",
                            "monitoring.resolution": "low"
                        },
                        "data_upload_start_date": "2025-01-15T10:30:00Z",
                        "memfaultd":  null
                    }
                },
                "revision": 42,
                "completed": 50
            }
        });

        verify_response("device_config_no_memfaultd", json, 42, Some(50));
    }

    #[test]
    fn test_device_config_with_memfaultd_empty_logs() {
        let json = json!({
            "data": {
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "high",
                            "logging.resolution": "normal",
                            "monitoring.resolution": "low"
                        },
                        "data_upload_start_date": "2025-01-15T10:30:00Z",
                        "memfaultd": {
                            "sdk-settings": {
                                "logs": null
                            }
                        }
                    }
                },
                "revision": 42,
                "completed": 50
            }
        });

        verify_response("device_cconfig_no_logs", json, 42, Some(50));
    }

    #[test]
    fn test_device_config_with_memfaultd_empty_sdk_settings() {
        let json = json!({
            "data": {
                "config": {
                    "memfault": {
                        "sampling": {
                            "debugging.resolution": "high",
                            "logging.resolution": "normal",
                            "monitoring.resolution": "low"
                        },
                        "data_upload_start_date": "2025-01-15T10:30:00Z",
                        "memfaultd": {
                            "sdk-settings": null
                        }
                    }
                },
                "revision": 42,
                "completed": 50
            }
        });

        verify_response("device_config_no_sdk_settings", json, 42, Some(50));
    }

    fn verify_response(test_name: &str, json_val: Value, revision: u32, completed: Option<u32>) {
        let json_string = json_val.to_string();
        let parsed: Result<DeviceConfigResponse, _> = serde_json::from_str(&json_string);

        assert!(parsed.is_ok());

        let response = parsed.unwrap();

        assert_eq!(response.data.revision, revision);
        assert_eq!(response.data.completed, completed);
        assert!(matches!(
            response.data.config.memfault.sampling.debugging_resolution,
            DeviceConfigResponseResolution::High
        ));

        assert_json_snapshot!(test_name, response);
    }
}
