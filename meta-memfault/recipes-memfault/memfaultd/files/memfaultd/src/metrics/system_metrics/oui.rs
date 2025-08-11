//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::str::FromStr;

use crate::metrics::metric_reading::KeyedMetricReading;
use crate::metrics::system_metrics::oui_parse::{
    get_active_wireless_ap_ouis, get_active_wireless_local_ouis,
};
use crate::metrics::system_metrics::SystemMetricFamilyCollector;
use crate::metrics::MetricStringKey;

pub struct OuiMetricsCollector;

impl OuiMetricsCollector {
    pub fn new() -> Self {
        Self
    }
}

impl SystemMetricFamilyCollector for OuiMetricsCollector {
    fn family_name(&self) -> &'static str {
        "wireless_oui"
    }

    fn collect_metrics(&mut self) -> eyre::Result<Vec<KeyedMetricReading>> {
        let mut readings = Vec::new();

        if let Some(local_ouis) = get_active_wireless_local_ouis() {
            for (interface, oui) in local_ouis {
                readings.push(KeyedMetricReading::new_report_tag(
                    MetricStringKey::from_str(&format!("wireless/oui/local_{}", interface))
                        .map_err(|e| eyre::eyre!("Couldn't construct metric key: {}", e))?,
                    oui,
                ));
            }
        }

        if let Some(ap_ouis) = get_active_wireless_ap_ouis() {
            for (interface, oui) in ap_ouis {
                readings.push(KeyedMetricReading::new_report_tag(
                    MetricStringKey::from_str(&format!("wireless/oui/ap_{}", interface))
                        .map_err(|e| eyre::eyre!("Couldn't construct metric key: {}", e))?,
                    oui,
                ));
            }
        }

        Ok(readings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_family_name() {
        let collector = OuiMetricsCollector::new();
        assert_eq!(collector.family_name(), "wireless_oui");
    }
}
