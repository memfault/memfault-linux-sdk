//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use eyre::Result;
use log::{debug, warn};

use crate::{
    config::SystemMetricConfig,
    metrics::KeyedMetricReading,
    util::system::{bytes_per_page, clock_ticks_per_second},
};

mod cpu;
use crate::metrics::system_metrics::cpu::{CpuMetricCollector, CPU_METRIC_NAMESPACE};

mod thermal;
use crate::metrics::system_metrics::thermal::{ThermalMetricsCollector, THERMAL_METRIC_NAMESPACE};

mod memory;
use crate::metrics::system_metrics::memory::{MemoryMetricsCollector, MEMORY_METRIC_NAMESPACE};

mod network_interfaces;
use crate::metrics::system_metrics::network_interfaces::{
    NetworkInterfaceMetricCollector, NetworkInterfaceMetricsConfig,
    NETWORK_INTERFACE_METRIC_NAMESPACE,
};

mod processes;
use processes::{ProcessMetricsCollector, PROCESSES_METRIC_NAMESPACE};

mod disk_space;
use disk_space::{
    DiskSpaceMetricCollector, DiskSpaceMetricsConfig, NixStatvfs, DISKSPACE_METRIC_NAMESPACE,
    DISKSPACE_METRIC_NAMESPACE_LEGACY,
};

use self::processes::ProcessMetricsConfig;
use super::MetricsMBox;

pub const BUILTIN_SYSTEM_METRIC_NAMESPACES: &[&str; 7] = &[
    CPU_METRIC_NAMESPACE,
    MEMORY_METRIC_NAMESPACE,
    THERMAL_METRIC_NAMESPACE,
    NETWORK_INTERFACE_METRIC_NAMESPACE,
    PROCESSES_METRIC_NAMESPACE,
    DISKSPACE_METRIC_NAMESPACE,
    // Include in list of namespaces so that
    // legacy collectd from the "df" plugin
    // are still filtered out
    DISKSPACE_METRIC_NAMESPACE_LEGACY,
];

pub trait SystemMetricFamilyCollector {
    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>>;
    fn family_name(&self) -> &'static str;
}

pub struct SystemMetricsCollector {
    metric_family_collectors: Vec<Box<dyn SystemMetricFamilyCollector>>,
    metrics_mbox: MetricsMBox,
}

impl SystemMetricsCollector {
    pub fn new(system_metric_config: SystemMetricConfig, metrics_mbox: MetricsMBox) -> Self {
        // CPU, Memory, and Thermal metrics are captured by default
        let mut metric_family_collectors: Vec<Box<dyn SystemMetricFamilyCollector>> = vec![
            Box::new(CpuMetricCollector::new()),
            Box::new(MemoryMetricsCollector::new()),
            Box::new(ThermalMetricsCollector::new()),
        ];

        // Check if process metrics have been manually configured
        match system_metric_config.processes {
            Some(processes) if !processes.is_empty() => {
                metric_family_collectors.push(Box::new(ProcessMetricsCollector::<Instant>::new(
                    ProcessMetricsConfig::Processes(processes),
                    clock_ticks_per_second() as f64 / 1000.0,
                    bytes_per_page() as f64,
                )))
            }
            // Monitoring no processes means this collector is disabled
            Some(_empty_set) => {}
            None => {
                metric_family_collectors.push(Box::new(ProcessMetricsCollector::<Instant>::new(
                    ProcessMetricsConfig::Auto,
                    clock_ticks_per_second() as f64 / 1000.0,
                    bytes_per_page() as f64,
                )))
            }
        };

        // Check if disk space metrics have been manually configured
        match system_metric_config.disk_space {
            Some(disks) if !disks.is_empty() => {
                metric_family_collectors.push(Box::new(DiskSpaceMetricCollector::new(
                    NixStatvfs::new(),
                    DiskSpaceMetricsConfig::Disks(disks),
                )))
            }
            // Monitoring no disks means this collector is disabled
            Some(_empty_set) => {}
            None => metric_family_collectors.push(Box::new(DiskSpaceMetricCollector::new(
                NixStatvfs::new(),
                DiskSpaceMetricsConfig::Auto,
            ))),
        };

        // Check if network interface metrics have been manually configured
        match system_metric_config.network_interfaces {
            Some(interfaces) if !interfaces.is_empty() => metric_family_collectors.push(Box::new(
                NetworkInterfaceMetricCollector::<Instant>::new(
                    NetworkInterfaceMetricsConfig::Interfaces(interfaces),
                ),
            )),
            // Monitoring no interfaces means this collector is disabled
            Some(_empty_set) => {}
            None => metric_family_collectors.push(Box::new(NetworkInterfaceMetricCollector::<
                Instant,
            >::new(
                NetworkInterfaceMetricsConfig::Auto
            ))),
        };

        Self {
            metric_family_collectors,
            metrics_mbox,
        }
    }

    pub fn run(&mut self, metric_poll_duration: Duration) {
        loop {
            for collector in self.metric_family_collectors.iter_mut() {
                match collector.collect_metrics() {
                    Ok(readings) => {
                        if let Err(e) = self.metrics_mbox.send_and_forget(readings) {
                            debug!(
                                "Couldn't add metric reading for family \"{}\": {:?}",
                                collector.family_name(),
                                e
                            )
                        }
                    }
                    Err(e) => warn!(
                        "Failed to collect readings for family \"{}\": {}",
                        collector.family_name(),
                        e
                    ),
                }
            }

            sleep(metric_poll_duration);
        }
    }
}
