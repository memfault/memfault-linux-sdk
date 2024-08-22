//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::PathBuf;

use eyre::Result;
use ssf::{Message, MsgMailbox};

use crate::network::NetworkConfig;

use super::{KeyedMetricReading, SessionName};

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
