//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{cli::MemfaultdClient, config::Config, metrics::KeyedMetricReading};

use eyre::Result;

pub fn write_metrics(metrics: Vec<KeyedMetricReading>, config: &Config) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    client.post_metrics(metrics)
}
