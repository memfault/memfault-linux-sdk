//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::config::Config;
use eyre::{eyre, Result};

use crate::cli::memfaultd_client::MemfaultdClient;

pub fn report_sync(config: &Config, success: bool) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    let status = if success { "successful" } else { "failed" };
    let command_string = if success {
        "report-sync-success "
    } else {
        "report-sync-failure"
    };

    match client.report_sync(success) {
        Ok(()) => {
            eprintln!("Reported a {} sync to memfaultd", status);
            Ok(())
        }
        Err(e) => Err(eyre!("{} failed: {:#}", command_string, e)),
    }
}
