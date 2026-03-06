//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use ssf::Message;

use crate::metrics::BatteryMonitorReading;

pub struct BatteryReadingMessage {
    pub reading: BatteryMonitorReading,
}

impl BatteryReadingMessage {
    pub fn new(reading: BatteryMonitorReading) -> Self {
        Self { reading }
    }
}

impl Message for BatteryReadingMessage {
    type Reply = ();
}
