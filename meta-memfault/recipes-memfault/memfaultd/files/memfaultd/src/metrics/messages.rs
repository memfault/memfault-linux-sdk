//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::Arc;

use eyre::Result;
use ssf::{Message, MsgMailbox};

use crate::{mar::MarConfig, network::NetworkConfig};

use super::{KeyedMetricReading, MetricReportType, SessionName};

/// Allows KeyedMetricReading to be sent as a message. The `ssf` framework will
/// automatically support sending `Vec<KeyedMetricReading>` as well.
impl Message for KeyedMetricReading {
    type Reply = Result<()>;
}

/// Syntactic Sugar because this is the most-used type of mailbox in memfaultd.
pub type MetricsMBox = MsgMailbox<Vec<KeyedMetricReading>>;

pub enum SessionEventMessage {
    StartSession {
        name: SessionName,
        readings: Vec<KeyedMetricReading>,
    },
    StopSession {
        name: SessionName,
        readings: Vec<KeyedMetricReading>,
        network_config: Arc<NetworkConfig>,
        mar_config: Arc<MarConfig>,
    },
}

impl Message for SessionEventMessage {
    type Reply = Result<()>;
}

#[derive(Clone)]
pub enum ReportsToDump {
    All,
    Report(MetricReportType),
}

#[derive(Clone)]
pub struct DumpMetricReportMessage {
    reports_to_dump: ReportsToDump,
    network_config: Arc<NetworkConfig>,
    mar_config: Arc<MarConfig>,
}

impl DumpMetricReportMessage {
    pub fn new(
        reports_to_dump: ReportsToDump,
        network_config: Arc<NetworkConfig>,
        mar_config: Arc<MarConfig>,
    ) -> Self {
        Self {
            reports_to_dump,
            network_config,
            mar_config,
        }
    }

    pub fn network_config(&self) -> &NetworkConfig {
        &self.network_config
    }

    pub fn reports_to_dump(&self) -> &ReportsToDump {
        &self.reports_to_dump
    }

    pub fn mar_config(&self) -> &MarConfig {
        &self.mar_config
    }
}
impl Message for DumpMetricReportMessage {
    type Reply = Result<()>;
}

#[derive(Clone)]
pub struct DumpHrtMessage {
    network_config: Arc<NetworkConfig>,
    mar_config: Arc<MarConfig>,
}

impl DumpHrtMessage {
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

impl Message for DumpHrtMessage {
    type Reply = Result<()>;
}
