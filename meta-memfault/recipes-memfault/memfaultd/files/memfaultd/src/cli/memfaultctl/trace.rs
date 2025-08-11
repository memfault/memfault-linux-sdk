//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::{
    cli::MemfaultdClient, config::Config, http_server::TraceRequest, mar::LinuxCustomTraceSource,
};
use eyre::Result;

pub fn save_trace(
    config: &Config,
    program: String,
    reason: String,
    crash: Option<bool>,
    source: Option<String>,
    signature: Option<String>,
) -> Result<()> {
    let client = MemfaultdClient::from_config(config)?;
    let source = if let Some(source) = source {
        LinuxCustomTraceSource::Other(source)
    } else {
        LinuxCustomTraceSource::Memfaultctl
    };
    client.save_trace(TraceRequest {
        signature,
        crash: crash.unwrap_or(false),
        reason,
        program,
        source,
        log_file_name: None,
    })
}
