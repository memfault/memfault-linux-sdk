//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{LogFilterRule, LogRuleAction},
    metrics::MetricReportType,
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

impl DeviceConfig {
    /// Overrides the sampling with the development sampling if `enable_dev_mode` is true.
    pub fn with_dev_mode(mut self, enable_dev_mode: bool) -> Self {
        if enable_dev_mode {
            self.sampling = Sampling::development();
        }
        self
    }

    // when no config is set default to capturing data
    pub fn capture_sampling(&self) -> Sampling {
        match self.revision {
            None => Sampling::development(),
            Some(_) => self.sampling,
        }
    }
}

impl Sampling {
    pub fn debugging_enabled(&self) -> bool {
        self.debugging_resolution >= Resolution::Normal
    }

    pub fn logging_enabled(&self) -> bool {
        self.logging_resolution >= Resolution::Normal
    }

    pub fn monitoring_enabled(&self) -> bool {
        self.monitoring_resolution >= Resolution::Normal
    }

    pub fn metric_report_enabled(&self, report_type: &MetricReportType) -> bool {
        match report_type {
            MetricReportType::DailyHeartbeat => self.monitoring_resolution >= Resolution::Low,
            MetricReportType::Heartbeat | MetricReportType::Session(_) => self.monitoring_enabled(),
        }
    }

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
    use std::str::FromStr;

    use insta::assert_json_snapshot;

    use crate::metrics::SessionName;

    use crate::network::{
        DeviceConfigResponseConfig, DeviceConfigResponseData, DeviceConfigResponseMemfault,
        DeviceConfigResponseSampling,
    };

    use super::*;

    #[rstest::rstest]
    #[case::not_fetched(None, Resolution::Off, true)]
    #[case::fetched_off(Some(1), Resolution::Off, false)]
    #[case::fetched_low(Some(1), Resolution::Low, false)]
    #[case::fetched_normal(Some(1), Resolution::Normal, true)]
    #[case::fetched_high(Some(1), Resolution::High, true)]
    fn test_debugging_capture_enabled(
        #[case] revision: Option<crate::network::DeviceConfigRevision>,
        #[case] debugging_resolution: Resolution,
        #[case] expected: bool,
    ) {
        let config = DeviceConfig {
            revision,
            sampling: Sampling {
                debugging_resolution,
                ..DeviceConfig::default().sampling
            },
            ..DeviceConfig::default()
        };
        assert_eq!(config.capture_sampling().debugging_enabled(), expected);
    }

    #[rstest::rstest]
    #[case::heartbeat_not_fetched(MetricReportType::Heartbeat, None, Resolution::Off, true)]
    #[case::heartbeat_off(MetricReportType::Heartbeat, Some(1), Resolution::Off, false)]
    #[case::heartbeat_low(MetricReportType::Heartbeat, Some(1), Resolution::Low, false)]
    #[case::heartbeat_normal(MetricReportType::Heartbeat, Some(1), Resolution::Normal, true)]
    #[case::heartbeat_high(MetricReportType::Heartbeat, Some(1), Resolution::High, true)]
    #[case::daily_heartbeat_not_fetched(
        MetricReportType::DailyHeartbeat,
        None,
        Resolution::Off,
        true
    )]
    #[case::daily_heartbeat_off(MetricReportType::DailyHeartbeat, Some(1), Resolution::Off, false)]
    #[case::daily_heartbeat_low(MetricReportType::DailyHeartbeat, Some(1), Resolution::Low, true)]
    #[case::daily_heartbeat_normal(
        MetricReportType::DailyHeartbeat,
        Some(1),
        Resolution::Normal,
        true
    )]
    #[case::daily_heartbeat_high(MetricReportType::DailyHeartbeat, Some(1), Resolution::High, true)]
    #[case::session_not_fetched(session(), None, Resolution::Off, true)]
    #[case::session_off(session(), Some(1), Resolution::Off, false)]
    #[case::session_low(session(), Some(1), Resolution::Low, false)]
    #[case::session_normal(session(), Some(1), Resolution::Normal, true)]
    #[case::session_high(session(), Some(1), Resolution::High, true)]
    fn test_metric_report_capture_enabled(
        #[case] report_type: MetricReportType,
        #[case] revision: Option<crate::network::DeviceConfigRevision>,
        #[case] monitoring_resolution: Resolution,
        #[case] expected: bool,
    ) {
        let config = DeviceConfig {
            revision,
            sampling: Sampling {
                monitoring_resolution,
                ..DeviceConfig::default().sampling
            },
            ..DeviceConfig::default()
        };
        assert_eq!(
            config
                .capture_sampling()
                .metric_report_enabled(&report_type),
            expected
        );
    }

    fn session() -> MetricReportType {
        MetricReportType::Session(SessionName::from_str("test").unwrap())
    }

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
