//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Serialize};

use crate::metrics::{KeyedMetricReading, SessionName};

#[derive(Serialize, Deserialize)]
pub struct SessionRequest {
    pub session_name: SessionName,
    pub readings: Vec<KeyedMetricReading>,
}

impl SessionRequest {
    pub fn new(session_name: SessionName, readings: Vec<KeyedMetricReading>) -> Self {
        Self {
            session_name,
            readings,
        }
    }

    pub fn new_without_readings(session_name: SessionName) -> Self {
        Self {
            session_name,
            readings: vec![],
        }
    }
}
