//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! An adapter to connect FluentBit to our LogCollector.
//!
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use log::warn;
use serde_json::{json, Value};

use crate::fluent_bit::{FluentdMessage, FluentdValue};

const ALWAYS_INCLUDE_KEYS: &[&str] = &["MESSAGE", "_PID", "_SYSTEMD_UNIT", "PRIORITY"];

/// An iterator that can be used as a source of logs for LogCollector.
/// Will filter fluent-bit messages to keep only log messages and convert them
/// to a serde_json::Value ready to be written to disk.
pub struct FluentBitAdapter {
    receiver: Receiver<FluentdMessage>,
    extra_fields: Vec<String>,
}

impl FluentBitAdapter {
    pub fn new(receiver: Receiver<FluentdMessage>, extra_fluent_bit_fields: &[String]) -> Self {
        Self {
            receiver,
            extra_fields: extra_fluent_bit_fields.to_owned(),
        }
    }

    /// Convert a FluentdMessage into a serde_json::Value that we can log.
    /// Returns None when this message should be filtered out.
    fn convert_message(msg: &FluentdMessage, extra_fields: &[String]) -> Option<Value> {
        if !msg.1.contains_key("MESSAGE") {
            // We are only interested in log messages. They will have a 'MESSAGE' key.
            // Metrics do not have the MESSAGE key.
            return None;
        }

        let data: HashMap<String, FluentdValue> = msg
            .1
            .iter()
            // Only keep some of the key/value pairs of the original message
            .filter_map(|(k, v)| match k {
                k if ALWAYS_INCLUDE_KEYS.contains(&k.as_str()) => Some((k.clone(), v.clone())),
                k if extra_fields.contains(k) => Some((k.clone(), v.clone())),
                _ => None,
            })
            .collect();

        Some(json!({
          "ts": msg.0.to_rfc3339(),
          "data": data
        }))
    }
}

impl Iterator for FluentBitAdapter {
    type Item = Value;
    /// Convert a FluentdMessage to a LogRecord for LogCollector.
    /// Messages can be filtered out by returning None here.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let msg_r = self.receiver.recv();
            match msg_r {
                Ok(msg) => {
                    let value = FluentBitAdapter::convert_message(&msg, &self.extra_fields);
                    match value {
                        v @ Some(_) => return v,
                        None => continue,
                    }
                }
                Err(e) => {
                    warn!("fluent-bit stopped receiving messages with error: {:?}", e);
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use rstest::{fixture, rstest};

    use super::*;
    use crate::fluent_bit::FluentdMessage;

    #[rstest]
    #[case("{}", "", "")]
    #[case(r#"{"MESSAGE":"TEST" }"#, r#"{"MESSAGE":"TEST"}"#, "")]
    #[case(
        r#"{"MESSAGE":"TEST", "SOME_EXTRA_KEY":"XX" }"#,
        r#"{"MESSAGE":"TEST"}"#,
        ""
    )]
    #[case(
        r#"{"MESSAGE":"TEST", "SOME_EXTRA_KEY":"XX", "_PID": "44", "_SYSTEMD_UNIT": "some.service", "PRIORITY": "6" }"#,
        r#"{"MESSAGE":"TEST","PRIORITY":"6","_PID":"44","_SYSTEMD_UNIT":"some.service"}"#,
         ""
    )]
    #[case(
        r#"{"MESSAGE":"TEST", "SOME_EXTRA_KEY":"XX" }"#,
        r#"{"MESSAGE":"TEST","SOME_EXTRA_KEY":"XX"}"#,
        "SOME_EXTRA_KEY"
    )]
    fn test_filtering(
        time: DateTime<Utc>,
        #[case] input: String,
        #[case] output: String,
        #[case] extras: String,
    ) {
        let m = FluentdMessage(time, serde_json::from_str(&input).unwrap());

        let extra_fluent_bit_attributes = extras.split(',').map(String::from).collect::<Vec<_>>();
        let r = FluentBitAdapter::convert_message(&m, &extra_fluent_bit_attributes);

        match r {
            Some(filtered) => {
                assert_eq!(
                    serde_json::to_string(&filtered.get("data")).unwrap(),
                    output
                );
            }
            None => assert!(output.is_empty()),
        }
    }

    #[fixture]
    fn time() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1334250000000).unwrap()
    }
}
