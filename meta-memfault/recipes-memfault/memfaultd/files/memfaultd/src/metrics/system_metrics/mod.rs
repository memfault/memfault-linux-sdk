//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::{Arc, Mutex};
use std::thread::{self, spawn};
use std::time::Duration;

use eyre::Result;
use log::warn;

use crate::metrics::MetricReportManager;
mod cpu;
use crate::metrics::system_metrics::cpu::{CpuMetricCollector, CPU_METRIC_NAMESPACE};

mod memory;
use crate::metrics::system_metrics::memory::{MemoryMetricsCollector, MEMORY_METRIC_NAMESPACE};

pub const BUILTIN_SYSTEM_METRIC_NAMESPACES: &[&str; 2] =
    &[CPU_METRIC_NAMESPACE, MEMORY_METRIC_NAMESPACE];

pub struct SystemMetricsCollector {}

impl SystemMetricsCollector {
    pub fn new() -> Self {
        Self {}
    }

    pub fn start(
        &self,
        metric_poll_duration: Duration,
        metric_report_manager: Arc<Mutex<MetricReportManager>>,
    ) -> Result<()> {
        let mut cpu_metric_collector = CpuMetricCollector::new();

        spawn(move || loop {
            thread::sleep(metric_poll_duration);

            match cpu_metric_collector.get_cpu_metrics() {
                Ok(cpu_metrics) => {
                    for metric_reading in cpu_metrics {
                        if let Err(e) = metric_report_manager
                            .lock()
                            .expect("Mutex poisoned")
                            .add_metric(metric_reading)
                        {
                            warn!("Couldn't add CPU metric: {}", e)
                        }
                    }
                }
                Err(e) => warn!("CPU metric collection failed: {}", e),
            }

            match MemoryMetricsCollector::get_memory_metrics() {
                Ok(memory_metric_readings) => {
                    for metric_reading in memory_metric_readings {
                        if let Err(e) = metric_report_manager
                            .lock()
                            .expect("Mutex poisoned")
                            .add_metric(metric_reading)
                        {
                            warn!("Couldn't add memory metric reading: {:?}", e)
                        }
                    }
                }
                Err(e) => warn!("Memory metric collection failed: {:?}", e),
            }
        });

        Ok(())
    }
}

impl Default for SystemMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}
