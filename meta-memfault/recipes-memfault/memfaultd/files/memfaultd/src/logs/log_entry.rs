//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::util::serialization::datetime_to_rfc3339;

const MESSAGE_KEY: &str = "MESSAGE";
const PID_KEY: &str = "_PID";
const SYSTEMD_UNIT_KEY: &str = "_SYSTEMD_UNIT";
const PRIORITY_KEY: &str = "PRIORITY";
const ORIGINAL_PRIORITY_KEY: &str = "ORIGINAL_PRIORITY";

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum LogValue {
    String(String),
    Float(f64),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
/// Represents the structured format of a log entry
///
/// Note that we will not serialize the fields that are `None` to save space.
pub struct LogData {
    #[serde(rename = "MESSAGE")]
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_PID")]
    pub pid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_SYSTEMD_UNIT")]
    pub systemd_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "PRIORITY")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "ORIGINAL_PRIORITY")]
    pub original_priority: Option<String>,
    #[serde(flatten)]
    pub extra_fields: HashMap<String, LogValue>,
}

impl LogData {
    /// Returns the value of the field with the given key.
    pub fn get_field(&self, key: &str) -> Option<String> {
        match key {
            MESSAGE_KEY => Some(self.message.clone()),
            PID_KEY => self.pid.clone(),
            SYSTEMD_UNIT_KEY => self.systemd_unit.clone(),
            PRIORITY_KEY => self.priority.clone(),
            ORIGINAL_PRIORITY_KEY => self.original_priority.clone(),
            _ => self.extra_fields.get(key).and_then(|v| match v {
                LogValue::String(s) => Some(s.clone()),
                LogValue::Float(_) => None,
            }),
        }
    }
}

/// Represents a structured log that could come from a variety of sources.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    #[serde(with = "datetime_to_rfc3339")]
    pub ts: DateTime<Utc>,
    pub data: LogData,
}

impl LogEntry {
    /// Filter log fields to only include defaults and those specified in `extra_fields`.
    ///
    /// This function modifies the log entry data to remove any extra fields that are not
    /// specified by the user.
    pub fn filter_extra_fields(&mut self, extra_fields: &[String]) {
        self.data
            .extra_fields
            .retain(|k, _| extra_fields.contains(k));
    }
}

#[cfg(test)]
impl LogEntry {
    pub fn new_with_message(message: &str) -> Self {
        LogEntry {
            ts: Utc::now(),
            data: LogData {
                message: message.to_string(),
                pid: None,
                systemd_unit: None,
                priority: None,
                original_priority: None,
                extra_fields: HashMap::new(),
            },
        }
    }

    pub fn new_with_message_and_ts(message: &str, ts: DateTime<Utc>) -> Self {
        LogEntry {
            ts,
            data: LogData {
                message: message.to_string(),
                pid: None,
                systemd_unit: None,
                priority: None,
                original_priority: None,
                extra_fields: HashMap::new(),
            },
        }
    }

    #[cfg(test)]
    pub fn new_with_message_level_and_service(message: &str, level: &str, service: &str) -> Self {
        let datetime = DateTime::parse_from_rfc3339("2004-06-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        LogEntry {
            ts: datetime,
            data: LogData {
                message: message.to_string(),
                pid: None,
                systemd_unit: Some(service.to_string()),
                priority: Some(level.to_string()),
                original_priority: None,
                extra_fields: HashMap::new(),
            },
        }
    }

    #[cfg(test)]
    pub fn new_with_message_and_extra_fields(
        message: &str,
        extra_fields: HashMap<String, LogValue>,
    ) -> Self {
        let datetime = DateTime::parse_from_rfc3339("2004-06-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        LogEntry {
            ts: datetime,
            data: LogData {
                message: message.to_string(),
                pid: None,
                systemd_unit: None,
                priority: None,
                original_priority: None,
                extra_fields,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use insta::{assert_json_snapshot, with_settings};
    use rstest::{fixture, rstest};

    use super::*;

    #[rstest]
    #[case("only_message", r#"{"MESSAGE":"TEST" }"#, "")]
    #[case("extra_key", r#"{"MESSAGE":"TEST", "SOME_EXTRA_KEY":"XX" }"#, "")]
    #[case(
        "multi_key_match",
        r#"{"MESSAGE":"TEST", "SOME_EXTRA_KEY":"XX", "_PID": "44", "_SYSTEMD_UNIT": "some.service", "PRIORITY": "6" }"#,
         ""
    )]
    #[case(
        "extra_attribute_filter",
        r#"{"MESSAGE":"TEST", "SOME_EXTRA_KEY":"XX" }"#,
        "SOME_EXTRA_KEY"
    )]
    fn test_filtering(
        time: DateTime<Utc>,
        #[case] test_name: String,
        #[case] input: String,
        #[case] extras: String,
    ) {
        let mut entry = LogEntry {
            ts: time,
            data: serde_json::from_str(&input).unwrap(),
        };

        let extra_attributes = extras.split(',').map(String::from).collect::<Vec<_>>();
        entry.filter_extra_fields(&extra_attributes);

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(test_name, entry);
        });
    }

    #[fixture]
    fn time() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1334250000000).unwrap()
    }
}
