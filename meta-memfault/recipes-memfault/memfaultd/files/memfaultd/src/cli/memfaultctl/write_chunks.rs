//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{cli::memfaultd_client::MemfaultdClient, config::Config};

use eyre::Result;

pub fn write_chunks(
    config: &Config,
    project_key: Option<String>,
    device_serial: Option<String>,
    chunks: Vec<String>,
) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    client.post_chunks(project_key, device_serial, chunks)
}
