//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! An adapter to connect FluentBit to our LogCollector.
//!
use std::sync::mpsc::Receiver;

use log::warn;

use crate::fluent_bit::FluentdMessage;
use crate::logs::log_entry::LogEntry;

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
    fn convert_message(msg: FluentdMessage, extra_fields: &[String]) -> Option<LogEntry> {
        if !msg.1.contains_key("MESSAGE") {
            // We are only interested in log messages. They will have a 'MESSAGE' key.
            // Metrics do not have the MESSAGE key.
            return None;
        }

        let mut log_entry = LogEntry::from(msg);
        log_entry.filter_fields(extra_fields);

        Some(log_entry)
    }
}

impl Iterator for FluentBitAdapter {
    type Item = LogEntry;
    /// Convert a FluentdMessage to a LogRecord for LogCollector.
    /// Messages can be filtered out by returning None here.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let msg_r = self.receiver.recv();
            match msg_r {
                Ok(msg) => {
                    let value = FluentBitAdapter::convert_message(msg, &self.extra_fields);
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
    use std::collections::HashMap;
    use std::sync::mpsc::channel;

    use chrono::{DateTime, NaiveDateTime, Utc};
    use insta::{assert_json_snapshot, with_settings};

    use super::*;
    use crate::fluent_bit::{FluentdMessage, FluentdValue};

    #[test]
    fn test_fluent_bit_adapter() {
        let (tx, rx) = channel();
        let adapter = FluentBitAdapter::new(rx, &[]);

        let mut map = HashMap::new();
        map.insert(
            "MESSAGE".to_string(),
            FluentdValue::String("test".to_string()),
        );
        let msg = FluentdMessage(time(), map);
        tx.send(msg).unwrap();

        let mut adapter_iter = adapter.into_iter();
        let log_entry = adapter_iter.next().unwrap();

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(log_entry);
        });
    }

    fn time() -> DateTime<Utc> {
        let naive = NaiveDateTime::from_timestamp_millis(1334250000000).unwrap();
        DateTime::<Utc>::from_utc(naive, Utc)
    }
}
