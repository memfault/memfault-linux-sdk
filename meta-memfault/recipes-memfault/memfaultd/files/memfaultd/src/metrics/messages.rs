//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::PathBuf;

use eyre::Result;
use ssf::{Message, MsgMailbox};

use crate::network::NetworkConfig;

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
        mar_staging_area: PathBuf,
        network_config: NetworkConfig,
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
    mar_staging_area: PathBuf,
    network_config: NetworkConfig,
}

impl DumpMetricReportMessage {
    pub fn new(
        reports_to_dump: ReportsToDump,
        mar_staging_area: PathBuf,
        network_config: NetworkConfig,
    ) -> Self {
        Self {
            reports_to_dump,
            mar_staging_area,
            network_config,
        }
    }

    pub fn mar_staging_area(&self) -> &PathBuf {
        &self.mar_staging_area
    }

    pub fn network_config(&self) -> &NetworkConfig {
        &self.network_config
    }

    pub fn reports_to_dump(&self) -> &ReportsToDump {
        &self.reports_to_dump
    }
}
impl Message for DumpMetricReportMessage {
    type Reply = Result<()>;
}

#[derive(Clone)]
pub struct DumpHrtMessage {
    mar_staging_area: PathBuf,
    network_config: NetworkConfig,
}

impl DumpHrtMessage {
    pub fn new(mar_staging_area: PathBuf, network_config: NetworkConfig) -> Self {
        Self {
            mar_staging_area,
            network_config,
        }
    }

    pub fn mar_staging_area(&self) -> &PathBuf {
        &self.mar_staging_area
    }

    pub fn network_config(&self) -> &NetworkConfig {
        &self.network_config
    }
}

impl Message for DumpHrtMessage {
    type Reply = Result<()>;
}
