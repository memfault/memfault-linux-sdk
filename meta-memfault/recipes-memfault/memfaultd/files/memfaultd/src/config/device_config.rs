//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{LogFilterRule, LogRuleAction},
    network::{
        DeviceConfigResponse, DeviceConfigResponseLogFilters, DeviceConfigResponseResolution,
        DeviceConfigRevision,
    },
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Resolution {
    Off,
    Low,
    Normal,
    High,
}

impl From<DeviceConfigResponseResolution> for Resolution {
    fn from(resolution: DeviceConfigResponseResolution) -> Self {
        match resolution {
            DeviceConfigResponseResolution::Off => Resolution::Off,
            DeviceConfigResponseResolution::Low => Resolution::Low,
            DeviceConfigResponseResolution::Normal => Resolution::Normal,
            DeviceConfigResponseResolution::High => Resolution::High,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct Sampling {
    pub debugging_resolution: Resolution,
    pub logging_resolution: Resolution,
    pub monitoring_resolution: Resolution,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceConfigLogging {
    pub filters: Option<DeviceConfigLoggingFilters>,
}

impl From<DeviceConfigResponseLogFilters> for DeviceConfigLogging {
    fn from(response: DeviceConfigResponseLogFilters) -> Self {
        Self {
            filters: Some(DeviceConfigLoggingFilters::from(response)),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceConfigLoggingFilters {
    pub default_action: Option<LogRuleAction>,
    pub log_filter_rules: Option<Vec<LogFilterRule>>,
}

impl From<DeviceConfigResponseLogFilters> for DeviceConfigLoggingFilters {
    fn from(response: DeviceConfigResponseLogFilters) -> Self {
        Self {
            default_action: response.default_action,
            log_filter_rules: response.log_filter_rules,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
/// DeviceConfig is configuration provided by Memfault backend.
pub struct DeviceConfig {
    pub revision: Option<DeviceConfigRevision>,
    pub sampling: Sampling,
    pub data_upload_start_date: Option<DateTime<Utc>>,
    pub logging: Option<DeviceConfigLogging>,
}

impl From<DeviceConfigResponse> for DeviceConfig {
    fn from(response: DeviceConfigResponse) -> Self {
        let logging = response
            .data
            .config
            .memfault
            .memfaultd
            .and_then(|memfaultd| memfaultd.sdk_settings)
            .and_then(|sdk_settings| sdk_settings.log_filters)
            .map(DeviceConfigLogging::from);

        Self {
            revision: Some(response.data.revision),
            sampling: Sampling {
                debugging_resolution: response
                    .data
                    .config
                    .memfault
                    .sampling
                    .debugging_resolution
                    .into(),
                logging_resolution: response
                    .data
                    .config
                    .memfault
                    .sampling
                    .logging_resolution
                    .into(),
                monitoring_resolution: response
                    .data
                    .config
                    .memfault
                    .sampling
                    .monitoring_resolution
                    .into(),
            },
            data_upload_start_date: response.data.config.memfault.data_upload_start_date,
            logging,
        }
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            revision: None,
            sampling: Sampling {
                debugging_resolution: Resolution::Off,
                logging_resolution: Resolution::Off,
                monitoring_resolution: Resolution::Off,
            },
            data_upload_start_date: None,
            logging: None,
        }
    }
}

impl Sampling {
    pub fn development() -> Self {
        Self {
            debugging_resolution: Resolution::High,
            logging_resolution: Resolution::High,
            monitoring_resolution: Resolution::High,
        }
    }
}

#[cfg(test)]
mod test {
    use insta::assert_json_snapshot;

    use crate::network::{
        DeviceConfigResponseConfig, DeviceConfigResponseData, DeviceConfigResponseMemfault,
        DeviceConfigResponseSampling,
    };

    use super::*;

    #[test]
    fn test_device_config_from_response() {
        let response = DeviceConfigResponse {
            data: DeviceConfigResponseData {
                config: DeviceConfigResponseConfig {
                    memfault: DeviceConfigResponseMemfault {
                        sampling: DeviceConfigResponseSampling {
                            debugging_resolution: DeviceConfigResponseResolution::High,
                            logging_resolution: DeviceConfigResponseResolution::High,
                            monitoring_resolution: DeviceConfigResponseResolution::High,
                        },
                        data_upload_start_date: Some(Utc::now()),
                        memfaultd: None,
                    },
                },
                revision: 42,
                completed: Some(100),
            },
        };

        let device_config: DeviceConfig = response.into();
        assert_json_snapshot!(device_config, {".data_upload_start_date" => "2025-11-24T16:02:39.340159894Z"});
    }
}
