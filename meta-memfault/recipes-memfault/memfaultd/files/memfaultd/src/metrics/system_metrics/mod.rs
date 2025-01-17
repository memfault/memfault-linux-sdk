//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashSet,
    thread::sleep,
    time::{Duration, Instant},
};

use eyre::{eyre, Result};
use log::debug;

use crate::{
    metrics::KeyedMetricReading,
    util::system::{bytes_per_page, clock_ticks_per_second},
};

mod cpu;
use crate::metrics::system_metrics::cpu::{CpuMetricCollector, CPU_METRIC_NAMESPACE};

mod thermal;
use thermal::ThermalMetricsCollector;
pub use thermal::THERMAL_METRIC_NAMESPACE;

mod memory;
use crate::metrics::system_metrics::memory::{MemoryMetricsCollector, MEMORY_METRIC_NAMESPACE};

mod network_interfaces;
use network_interfaces::{NetworkInterfaceMetricCollector, NetworkInterfaceMetricsConfig};
pub use network_interfaces::{
    METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX, METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX,
    NETWORK_INTERFACE_METRIC_NAMESPACE,
};

mod processes;
pub use processes::ProcessMetricsConfig;
use processes::{ProcessMetricsCollector, PROCESSES_METRIC_NAMESPACE};

mod disk_space;
pub use disk_space::DiskSpaceMetricsConfig;
use disk_space::{
    DiskSpaceMetricCollector, NixStatvfs, DISKSPACE_METRIC_NAMESPACE,
    DISKSPACE_METRIC_NAMESPACE_LEGACY,
};

mod diskstats;
pub use diskstats::DiskstatsMetricsConfig;
use diskstats::{DiskstatsMetricCollector, DISKSTATS_METRIC_NAMESPACE};

use self::memory::{MemInfoParser, MemInfoParserImpl};
use super::MetricsMBox;

pub const BUILTIN_SYSTEM_METRIC_NAMESPACES: &[&str; 8] = &[
    CPU_METRIC_NAMESPACE,
    MEMORY_METRIC_NAMESPACE,
    THERMAL_METRIC_NAMESPACE,
    NETWORK_INTERFACE_METRIC_NAMESPACE,
    PROCESSES_METRIC_NAMESPACE,
    DISKSPACE_METRIC_NAMESPACE,
    DISKSTATS_METRIC_NAMESPACE,
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
    pub fn new(
        processes_config: ProcessMetricsConfig,
        network_interfaces_config: Option<HashSet<String>>,
        disk_space_config: DiskSpaceMetricsConfig,
        diskstats_config: DiskstatsMetricsConfig,
        metrics_mbox: MetricsMBox,
    ) -> Self {
        // CPU, Memory, and Thermal metrics are captured by default
        let mut metric_family_collectors: Vec<Box<dyn SystemMetricFamilyCollector>> = vec![
            Box::new(CpuMetricCollector::new()),
            Box::new(MemoryMetricsCollector::new(MemInfoParserImpl::new())),
            Box::new(ThermalMetricsCollector::new()),
        ];

        // Check if process metrics have been manually configured
        match processes_config {
            // Monitoring no processes means this collector is disabled
            ProcessMetricsConfig::Processes(processes) if processes.is_empty() => {}
            // In all other cases we can just directly pass the config to ProcessMetricsCollector
            process_metrics_config => {
                // We need the total memory for the system to calculate the
                // percent used by each individual process
                if let Ok(mem_total) = Self::get_total_memory() {
                    metric_family_collectors.push(Box::new(
                        ProcessMetricsCollector::<Instant>::new(
                            process_metrics_config,
                            clock_ticks_per_second() as f64 / 1000.0,
                            bytes_per_page() as f64,
                            mem_total,
                        ),
                    ))
                }
            }
        };

        // Check if disk space metrics have been manually configured
        match disk_space_config {
            // Monitoring no disks means this collector is disabled
            DiskSpaceMetricsConfig::Disks(disks) if disks.is_empty() => {}
            disk_space_metrics_config => metric_family_collectors.push(Box::new(
                DiskSpaceMetricCollector::new(NixStatvfs::new(), disk_space_metrics_config),
            )),
        };

        // Check if diskstats metrics have been manually configured
        match diskstats_config {
            // Monitoring no devices means this collector is disabled
            DiskstatsMetricsConfig::Devices(devices) if devices.is_empty() => {}
            diskstats_config => metric_family_collectors.push(Box::new(
                DiskstatsMetricCollector::<Instant>::new(diskstats_config),
            )),
        };

        // Check if network interface metrics have been manually configured
        match network_interfaces_config {
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

    fn get_total_memory() -> Result<f64> {
        let mem_info_parser = MemInfoParserImpl::new();
        let mut stats = mem_info_parser.get_meminfo_stats()?;
        stats
            .remove("MemTotal")
            .ok_or_else(|| eyre!("Couldn't get MemTotal"))
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
                    Err(e) => debug!(
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
