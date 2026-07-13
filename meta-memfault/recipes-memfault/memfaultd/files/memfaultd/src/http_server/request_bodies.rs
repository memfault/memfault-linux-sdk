//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Serialize};

use crate::{
    mar::{LinuxCustomTraceSource, TraceLocals},
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
pub struct ChunksRequest {
    pub project_key: Option<String>,
    pub device_serial: Option<String>,
    pub chunks: Vec<String>,
}

impl ChunksRequest {
    pub fn new(
        project_key: Option<String>,
        device_serial: Option<String>,
        chunks: Vec<String>,
    ) -> Self {
        Self {
            project_key,
            device_serial,
            chunks,
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
    /// Arbitrary scalar key-value pairs attached to the trace as top-level
    /// metadata. Not used as an input to issue signature grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locals: Option<TraceLocals>,
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
            locals: None,
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

    #[test]
    fn test_trace_request_with_locals_roundtrip() {
        let body = r#"{
            "signature": "update()",
            "crash": true,
            "reason": "UpdateError",
            "program": "ota.py",
            "source": "MEMFAULTCTL",
            "locals": {"resource-id": "3411a39c", "retries": 3, "fatal": true}
        }"#;

        let request: TraceRequest = serde_json::from_str(body).expect("Failed to parse body");
        let locals = request.locals.expect("locals should be parsed");

        // Locals serialize back out as a top-level object, untouched.
        let value = serde_json::to_value(&locals).unwrap();
        assert_eq!(value["resource-id"], "3411a39c");
        assert_eq!(value["retries"], 3);
        assert_eq!(value["fatal"], true);
    }

    #[test]
    fn test_trace_request_rejects_non_scalar_locals() {
        let body = r#"{
            "crash": false,
            "reason": "r",
            "program": "p",
            "source": "MEMFAULTCTL",
            "locals": {"nested": {"a": 1}}
        }"#;

        assert!(serde_json::from_str::<TraceRequest>(body).is_err());
    }
}
