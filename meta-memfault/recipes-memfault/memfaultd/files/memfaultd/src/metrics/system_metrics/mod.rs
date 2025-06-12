//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    path::PathBuf,
    thread::sleep,
    time::{Duration, Instant},
};

use disk::{get_tracked_disks, DiskMetricsCollector};
use eyre::{eyre, Result};
use log::{debug, error};

use crate::{
    metrics::KeyedMetricReading,
    mmc::MmcImpl,
    util::system::{bytes_per_page, clock_ticks_per_second},
};

mod config;
pub use config::{CpuMetricsConfig, MemoryMetricsConfig, SystemMetricConfig, ThermalMetricsConfig};

mod cpu;
use crate::metrics::system_metrics::cpu::CpuMetricCollector;
pub use crate::metrics::system_metrics::cpu::CPU_METRIC_NAMESPACE;

mod thermal;
use thermal::ThermalMetricsCollector;
pub use thermal::THERMAL_METRIC_NAMESPACE;

mod memory;
use crate::metrics::system_metrics::memory::MemoryMetricsCollector;
pub use crate::metrics::system_metrics::memory::MEMORY_METRIC_NAMESPACE;

mod network_interfaces;
use network_interfaces::{NetworkInterfaceMetricCollector, NetworkInterfaceMetricsConfig};
pub use network_interfaces::{
    METRIC_INTERFACE_BYTES_PER_SECOND_RX_SUFFIX, METRIC_INTERFACE_BYTES_PER_SECOND_TX_SUFFIX,
    NETWORK_INTERFACE_METRIC_NAMESPACE,
};

mod processes;
use processes::ProcessMetricsCollector;
pub use processes::ProcessMetricsConfig;
pub use processes::PROCESSES_METRIC_NAMESPACE;

mod disk;

mod disk_space;
use disk_space::{DiskSpaceMetricCollector, NixStatvfs};
pub use disk_space::{DiskSpaceMetricsConfig, DISKSPACE_METRIC_NAMESPACE_LEGACY};

mod diskstats;
use diskstats::DiskstatsMetricCollector;
pub use diskstats::{DiskstatsMetricsConfig, DISKSTATS_METRIC_NAMESPACE};

use self::{
    memory::{MemInfoParser, MemInfoParserImpl},
    processes::ProcfsProcessNameMapper,
};
use super::MetricsMBox;

pub trait SystemMetricFamilyCollector {
    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>>;
    fn family_name(&self) -> &'static str;
}
pub struct SystemMetricsCollector {
    metric_family_collectors: Vec<Box<dyn SystemMetricFamilyCollector>>,
    metrics_mbox: MetricsMBox,
    poll_interval: Duration,
}

impl SystemMetricsCollector {
    pub fn new(config: SystemMetricConfig, metrics_mbox: MetricsMBox) -> Self {
        let mut metric_family_collectors: Vec<Box<dyn SystemMetricFamilyCollector>> = vec![];

        if config.cpu_metrics_enabled() {
            metric_family_collectors.push(Box::new(CpuMetricCollector::new()));
        }

        if config.memory_metrics_enabled() {
            metric_family_collectors.push(Box::new(MemoryMetricsCollector::new(
                MemInfoParserImpl::new(),
            )));
        }

        if config.thermal_metrics_enabled() {
            metric_family_collectors.push(Box::new(ThermalMetricsCollector::new()));
        }

        // Check if process metrics have been manually configured
        match config.monitored_processes() {
            // Monitoring no processes means this collector is disabled
            ProcessMetricsConfig::Processes(processes) if processes.is_empty() => {}
            // In all other cases we can just directly pass the config to ProcessMetricsCollector
            process_metrics_config => {
                // We need the total memory for the system to calculate the
                // percent used by each individual process
                if let Ok(mem_total) = Self::get_total_memory() {
                    metric_family_collectors.push(Box::new(ProcessMetricsCollector::<
                        Instant,
                        ProcfsProcessNameMapper,
                    >::new(
                        process_metrics_config,
                        clock_ticks_per_second() as f64 / 1000.0,
                        bytes_per_page() as f64,
                        mem_total,
                    )))
                }
            }
        };

        // Check if disk space metrics have been manually configured
        match config.disk_space_config() {
            // Monitoring no disks means this collector is disabled
            DiskSpaceMetricsConfig::Disks(disks) if disks.is_empty() => {}
            disk_space_metrics_config => metric_family_collectors.push(Box::new(
                DiskSpaceMetricCollector::new(NixStatvfs::new(), disk_space_metrics_config),
            )),
        };

        // Check if diskstats metrics have been manually configured
        match config.diskstats_config() {
            // Monitoring no devices means this collector is disabled
            DiskstatsMetricsConfig::Devices(devices) if devices.is_empty() => {}
            diskstats_config => metric_family_collectors.push(Box::new(
                DiskstatsMetricCollector::<Instant>::new(diskstats_config),
            )),
        };

        // TODO: Implement actual config values for disk metrics
        match config.diskstats_config() {
            // Monitoring no devices means this collector is disabled
            DiskstatsMetricsConfig::Devices(devices) if devices.is_empty() => {}
            diskstats_config => match get_tracked_disks(diskstats_config, "/sys/block") {
                Ok(disks) => {
                    let mmc = disks
                        .into_iter()
                        .filter_map(|disk_path| match MmcImpl::new(PathBuf::from(disk_path)) {
                            Ok(mmc) => Some(mmc),
                            Err(e) => {
                                debug!("Failed to open disk: {}", e);
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    metric_family_collectors.push(Box::new(DiskMetricsCollector::new(mmc)));
                }
                Err(e) => error!("Failed to start disk metrics collector: {}", e),
            },
        };

        // Check if network interface metrics have been manually configured
        match config.network_interfaces_config() {
            Some(interfaces) if !interfaces.is_empty() => metric_family_collectors.push(Box::new(
                NetworkInterfaceMetricCollector::<Instant>::new(
                    NetworkInterfaceMetricsConfig::Interfaces(interfaces.clone()),
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

        let poll_interval = config.poll_interval();

        Self {
            metric_family_collectors,
            metrics_mbox,
            poll_interval,
        }
    }

    fn get_total_memory() -> Result<f64> {
        let mem_info_parser = MemInfoParserImpl::new();
        let mut stats = mem_info_parser.get_meminfo_stats()?;
        stats
            .remove("MemTotal")
            .ok_or_else(|| eyre!("Couldn't get MemTotal"))
    }

    pub fn run(&mut self) {
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

            sleep(self.poll_interval);
        }
    }
}
