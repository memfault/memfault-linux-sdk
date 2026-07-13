//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::Arc;

use eyre::Result;
use ssf::Message;

use crate::{http_server::ChunksRequest, mar::MarConfig, network::NetworkConfig};

pub struct RelayChunksMsg {
    pub device_serial: Option<String>,
    pub project_key: Option<String>,
    pub chunks: Vec<String>,
}

impl RelayChunksMsg {
    pub fn new(
        device_serial: Option<String>,
        project_key: Option<String>,
        chunks: Vec<String>,
    ) -> Self {
        Self {
            device_serial,
            project_key,
            chunks,
        }
    }
}

impl Message for RelayChunksMsg {
    type Reply = Result<()>;
}

impl From<&ChunksRequest> for RelayChunksMsg {
    fn from(value: &ChunksRequest) -> Self {
        Self {
            device_serial: value.device_serial.clone(),
            project_key: value.project_key.clone(),
            chunks: value.chunks.clone(),
        }
    }
}

pub struct PrepareMarEntriesMsg {
    network_config: Arc<NetworkConfig>,
    mar_config: Arc<MarConfig>,
}

impl PrepareMarEntriesMsg {
    pub fn new(network_config: Arc<NetworkConfig>, mar_config: Arc<MarConfig>) -> Self {
        Self {
            network_config,
            mar_config,
        }
    }

    pub fn network_config(&self) -> &NetworkConfig {
        &self.network_config
    }

    pub fn mar_config(&self) -> &MarConfig {
        &self.mar_config
    }
}

impl Message for PrepareMarEntriesMsg {
    type Reply = Result<()>;
}
