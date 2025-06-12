//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{collections::HashSet, time::Duration};

use serde::{Deserialize, Serialize};

use crate::util::serialization::seconds_to_duration;

use super::{DiskSpaceMetricsConfig, DiskstatsMetricsConfig, ProcessMetricsConfig};
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SystemMetricConfig {
    pub enable: bool,
    #[serde(with = "seconds_to_duration")]
    pub poll_interval_seconds: Duration,
    pub processes: Option<HashSet<String>>,
    pub disk_space: Option<HashSet<String>>,
    pub diskstats: Option<HashSet<String>>,
    pub network_interfaces: Option<HashSet<String>>,
    pub cpu: Option<CpuMetricsConfig>,
    pub memory: Option<MemoryMetricsConfig>,
    pub thermal: Option<ThermalMetricsConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CpuMetricsConfig {
    pub enable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemoryMetricsConfig {
    pub enable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ThermalMetricsConfig {
    pub enable: bool,
}

impl SystemMetricConfig {
    pub fn cpu_metrics_enabled(&self) -> bool {
        match self.cpu {
            None => true,
            Some(CpuMetricsConfig { enable }) => enable,
        }
    }

    pub fn memory_metrics_enabled(&self) -> bool {
        match self.memory {
            None => true,
            Some(MemoryMetricsConfig { enable }) => enable,
        }
    }

    pub fn thermal_metrics_enabled(&self) -> bool {
        match self.thermal {
            None => true,
            Some(ThermalMetricsConfig { enable }) => enable,
        }
    }

    pub fn diskstats_metrics_enabled(&self) -> bool {
        !self
            .diskstats
            .as_ref()
            .is_some_and(|disks| disks.is_empty())
    }

    pub fn disk_space_metrics_enabled(&self) -> bool {
        !self
            .disk_space
            .as_ref()
            .is_some_and(|disks| disks.is_empty())
    }

    pub fn process_metrics_enabled(&self) -> bool {
        !self
            .processes
            .as_ref()
            .is_some_and(|processes| processes.is_empty())
    }

    pub fn network_metrics_enabled(&self) -> bool {
        !self
            .network_interfaces
            .as_ref()
            .is_some_and(|interfaces| interfaces.is_empty())
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval_seconds
    }

    pub fn monitored_processes(&self) -> ProcessMetricsConfig {
        match self.processes.as_ref() {
            Some(processes) => {
                let mut processes = processes.clone();
                processes.insert("memfaultd".to_string());
                ProcessMetricsConfig::Processes(processes)
            }
            None => ProcessMetricsConfig::Auto,
        }
    }

    pub fn disk_space_config(&self) -> DiskSpaceMetricsConfig {
        match self.disk_space.as_ref() {
            Some(mounts) => DiskSpaceMetricsConfig::Disks(mounts.clone()),
            None => DiskSpaceMetricsConfig::Auto,
        }
    }

    pub fn diskstats_config(&self) -> DiskstatsMetricsConfig {
        match self.diskstats.as_ref() {
            Some(devices) => DiskstatsMetricsConfig::Devices(devices.clone()),
            None => DiskstatsMetricsConfig::Auto,
        }
    }

    pub fn network_interfaces_config(&self) -> Option<&HashSet<String>> {
        self.network_interfaces.as_ref()
    }
}
