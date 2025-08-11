//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Serialize};

use crate::{
    mar::LinuxCustomTraceSource,
    metrics::{KeyedMetricReading, SessionName},
};

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

#[derive(Serialize, Deserialize)]
pub struct MetricsRequest {
    pub readings: Vec<KeyedMetricReading>,
}

impl MetricsRequest {
    pub fn new(readings: Vec<KeyedMetricReading>) -> Self {
        Self { readings }
    }
}

#[derive(Serialize, Deserialize)]
pub struct TraceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    pub crash: bool,
    pub reason: String,
    pub program: String,
    pub source: LinuxCustomTraceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_file_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_args_json_roundtrip() {
        let mut trace_args = TraceRequest {
            signature: Some("test_signature".to_string()),
            crash: true,
            reason: "test_reason".to_string(),
            program: "test_program".to_string(),
            source: LinuxCustomTraceSource::MemfaultWatch,
            log_file_name: Some("test_log".to_string()),
        };

        let json = serde_json::to_string(&trace_args).expect("Failed to serialize to JSON");
        insta::assert_snapshot!(json);

        trace_args.source = LinuxCustomTraceSource::Memfaultctl;
        let json_2 = serde_json::to_string(&trace_args).expect("Failed to serialize to JSON");
        insta::assert_snapshot!(json_2);

        trace_args.source = LinuxCustomTraceSource::Other("PYFAULT".to_string());
        let json_3 = serde_json::to_string(&trace_args).expect("Failed to serialize to JSON");
        insta::assert_snapshot!(json_3);
    }
}
