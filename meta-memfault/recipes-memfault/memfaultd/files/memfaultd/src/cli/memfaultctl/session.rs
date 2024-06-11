//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};

use crate::config::Config;
use crate::{
    cli::memfaultd_client::MemfaultdClient,
    metrics::{KeyedMetricReading, SessionName},
};

pub fn start_session(
    config: &Config,
    session_name: SessionName,
    metric_readings: Vec<KeyedMetricReading>,
) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    if config.config_file.enable_data_collection {
        match client.start_session(session_name.clone(), metric_readings) {
            Ok(()) => {
                eprintln!("Started new {} session", session_name);
                Ok(())
            }
            Err(e) => Err(eyre!("start-session failed: {:?}", e)),
        }
    } else {
        Err(eyre!(
            "Cannot start session with data collection disabled.\n\
             You can enable data collection with \"memfaultctl enable-data-collection\""
        ))
    }
}

pub fn end_session(
    config: &Config,
    session_name: SessionName,
    readings: Vec<KeyedMetricReading>,
) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    if config.config_file.enable_data_collection {
        match client.end_session(session_name.clone(), readings) {
            Ok(()) => {
                eprintln!("Ended ongoing {} session", session_name);
                Ok(())
            }
            Err(e) => Err(eyre!("end-session failed: {:?}", e)),
        }
    } else {
        Err(eyre!(
            "Cannot end session with data collection disabled.\n\
             You can enable data collection with \"memfaultctl enable-data-collection\""
        ))
    }
}
