//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Serialize};

use crate::network::{DeviceConfigResponse, DeviceConfigResponseResolution, DeviceConfigRevision};

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
/// DeviceConfig is configuration provided by Memfault backend.
pub struct DeviceConfig {
    pub revision: Option<DeviceConfigRevision>,
    pub sampling: Sampling,
}

impl From<DeviceConfigResponse> for DeviceConfig {
    fn from(response: DeviceConfigResponse) -> Self {
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
