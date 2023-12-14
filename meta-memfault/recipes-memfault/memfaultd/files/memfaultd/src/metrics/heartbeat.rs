//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Utc;
use eyre::{eyre, Result};
use log::debug;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    mar::{MarEntryBuilder, Metadata},
    metrics::{MetricReading, MetricStringKey, MetricValue},
    network::NetworkConfig,
};

mod timeseries;
use self::timeseries::{Counter, Histogram, TimeSeries, TimeWeightedAverage};

use super::{battery::METRIC_BATTERY_SOC_PCT, metric_reading::KeyedMetricReading};

pub struct HeartbeatManager {
    metrics: HashMap<MetricStringKey, Box<dyn TimeSeries + Send>>,
    start: Instant,
}

struct HeartbeatSnapshot {
    duration: Duration,
    metrics: HashMap<MetricStringKey, MetricValue>,
}

impl HeartbeatManager {
    pub fn new() -> Self {
        HeartbeatManager {
            metrics: HashMap::new(),
            start: Instant::now(),
        }
    }

    pub fn add_metric(&mut self, m: KeyedMetricReading) -> Result<()> {
        match self.metrics.entry(m.name) {
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let key = o.key().clone();
                let state = o.get_mut();
                if let Err(e) = (*state).aggregate(&m.value) {
                    *state = Self::select_aggregate_for(&key, &m.value)?;
                    log::warn!(
                        "New value for metric {} is incompatible ({}). Resetting timeseries.",
                        o.key(),
                        e
                    );
                }
            }
            std::collections::hash_map::Entry::Vacant(v) => {
                let timeseries = Self::select_aggregate_for(v.key(), &m.value)?;
                v.insert(timeseries);
            }
        };
        Ok(())
    }

    /// Increment a counter metric by 1
    pub fn increment_counter(&mut self, name: &str) -> Result<()> {
        self.add_to_counter(name, 1.0)
    }

    pub fn add_to_counter(&mut self, name: &str, value: f64) -> Result<()> {
        match name.parse::<MetricStringKey>() {
            Ok(metric_name) => self.add_metric(KeyedMetricReading::new(
                metric_name,
                MetricReading::Counter {
                    value,
                    timestamp: Utc::now(),
                },
            )),
            Err(e) => Err(eyre!("Invalid metric name: {} - {}", name, e)),
        }
    }

    /// Return all the metrics in memory and resets the store.
    pub fn take_metrics(&mut self) -> HashMap<MetricStringKey, MetricValue> {
        self.take_heartbeat_snapshot().metrics
    }

    fn take_heartbeat_snapshot(&mut self) -> HeartbeatSnapshot {
        let duration = std::mem::replace(&mut self.start, Instant::now()).elapsed();
        let metrics = std::mem::take(&mut self.metrics)
            .into_iter()
            .map(|(name, state)| (name, state.value()))
            .collect();

        HeartbeatSnapshot { duration, metrics }
    }

    /// Create one heartbeat entry with all the metrics in the store.
    /// All data will be timestamped with current time measured by CollectionTime::now(), effectively
    /// disregarding the collectd timestamps.
    fn prepare_heartbeat(
        &mut self,
        mar_staging_area: &Path,
    ) -> Result<Option<MarEntryBuilder<Metadata>>> {
        let snapshot = self.take_heartbeat_snapshot();

        if snapshot.metrics.is_empty() {
            return Ok(None);
        }

        Ok(Some(MarEntryBuilder::new(mar_staging_area)?.set_metadata(
            Metadata::LinuxHeartbeat {
                metrics: snapshot.metrics,
                duration: snapshot.duration,
            },
        )))
    }

    /// Dump the metrics to a MAR entry. This takes a &Arc<Mutex<MetricStore>>
    /// and will minimize lock time.
    /// This will empty the metrics store.
    pub fn dump_heartbeat_manager_to_mar_entry(
        heartbeat_manager: &Arc<Mutex<Self>>,
        mar_staging_area: &Path,
        network_config: &NetworkConfig,
    ) -> Result<()> {
        // Lock the store only long enough to create the HashMap
        let mar_builder = heartbeat_manager
            .lock()
            .unwrap()
            .prepare_heartbeat(mar_staging_area)?;

        // Save to disk after releasing the lock
        if let Some(mar_builder) = mar_builder {
            let mar_entry = mar_builder
                .save(network_config)
                .map_err(|e| eyre!("Error building MAR entry: {}", e))?;
            debug!(
                "Generated MAR entry from metrics: {}",
                mar_entry.path.display()
            );
        } else {
            debug!("Skipping generating metrics entry. No metrics in store.")
        }
        Ok(())
    }

    fn select_aggregate_for(
        key: &MetricStringKey,
        event: &MetricReading,
    ) -> Result<Box<dyn TimeSeries + Send>> {
        match event {
            MetricReading::Gauge { .. } => {
                // Very basic heuristic for now - in the future we may want to make this user-configurable.
                if key.as_str().eq(METRIC_BATTERY_SOC_PCT) {
                    Ok(Box::new(TimeWeightedAverage::new(event)?))
                } else {
                    Ok(Box::new(Histogram::new(event)?))
                }
            }
            MetricReading::Counter { .. } => Ok(Box::new(Counter::new(event)?)),
        }
    }
}

impl Default for HeartbeatManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;
    use crate::metrics::MetricTimestamp;
    use chrono::Duration;
    use std::str::FromStr;

    #[rstest]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("bar", 1000, 2.0), ("baz", 1000, 3.0)]), "foo", 1.0)]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 3.0)]), "foo", 2.0)]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 1.0)]), "foo",  1.0)]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0)]), "foo", 1.5)]
    #[case(in_gauges(vec![("foo", 1000, 1.0), ("foo", 1000, 2.0), ("foo", 1000, 2.0)]), "foo",  5.0/3.0)]
    fn test_aggregate_metrics(
        #[case] metrics: impl Iterator<Item = KeyedMetricReading>,
        #[case] name: &str,
        #[case] expected: f64,
    ) {
        let mut store = HeartbeatManager::new();

        for m in metrics {
            store.add_metric(m).unwrap();
        }
        let h = store.take_metrics();
        match h.get(&MetricStringKey::from_str(name).unwrap()).unwrap() {
            MetricValue::Number(e) => assert_eq!(*e, expected),
        }
    }

    #[rstest]
    fn test_empty_after_write() {
        let mut store = HeartbeatManager::new();
        for m in in_gauges(vec![
            ("foo", 1000, 1.0),
            ("bar", 1000, 2.0),
            ("baz", 1000, 3.0),
        ]) {
            store.add_metric(m).unwrap();
        }

        let tempdir = TempDir::new().unwrap();
        let _ = store.prepare_heartbeat(tempdir.path());
        assert_eq!(store.take_metrics().len(), 0);
    }

    fn in_gauges(
        metrics: Vec<(&'static str, i64, f64)>,
    ) -> impl Iterator<Item = KeyedMetricReading> {
        metrics
            .into_iter()
            .enumerate()
            .map(|(i, (name, interval, value))| KeyedMetricReading {
                name: MetricStringKey::from_str(name).unwrap(),
                value: MetricReading::Gauge {
                    value,
                    interval: Duration::milliseconds(interval),
                    timestamp: MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap()
                        + chrono::Duration::seconds(i as i64),
                },
            })
    }
}
