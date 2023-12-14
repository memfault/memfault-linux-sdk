//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::config::Config;
use eyre::{eyre, Result};

use crate::cli::memfaultd_client::MemfaultdClient;

pub fn add_battery_reading(config: &Config, reading_string: &str) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;

    match client.add_battery_reading(reading_string) {
        Ok(()) => {
            eprintln!("Successfully published battery reading to memfaultd");
            Ok(())
        }
        Err(e) => Err(eyre!("add-battery-reading failed: {:#}", e)),
    }
}
