//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::HashMap;

use chrono::{DateTime, Utc};

mod metric_string_key;
pub use metric_string_key::MetricStringKey;

mod metric_store;
pub use metric_store::InMemoryMetricStore;

mod metric_reading;
pub use metric_reading::KeyedMetricReading;
pub use metric_reading::MetricReading;

mod metric_value;
pub use metric_value::MetricValue;

use crate::mar::Metadata;

pub type MetricTimestamp = DateTime<Utc>;

impl From<HashMap<MetricStringKey, MetricValue>> for Metadata {
    fn from(metrics: HashMap<MetricStringKey, MetricValue>) -> Self {
        Metadata::LinuxHeartbeat { metrics }
    }
}
