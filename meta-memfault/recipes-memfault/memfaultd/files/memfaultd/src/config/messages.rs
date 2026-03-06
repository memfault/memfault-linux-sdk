//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::Arc;

use ssf::Message;

use crate::config::DeviceConfig;

#[derive(Clone)]
pub struct DeviceConfigUpdateMessage {
    pub config: Arc<DeviceConfig>,
}

impl Message for DeviceConfigUpdateMessage {
    type Reply = ();
}
