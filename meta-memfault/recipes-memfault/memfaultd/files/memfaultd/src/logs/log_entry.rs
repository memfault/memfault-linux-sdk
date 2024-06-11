//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::util::serialization::datetime_to_rfc3339;

// journal fields that should always be captured by memfaultd:
// https://man7.org/linux/man-pages/man7/systemd.journal-fields.7.html
const ALWAYS_INCLUDE_KEYS: &[&str] = &["MESSAGE", "_PID", "_SYSTEMD_UNIT", "PRIORITY"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogValue {
    String(String),
    Float(f64),
}

/// Represents a structured log that could come from a variety of sources.
#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntry {
    #[serde(with = "datetime_to_rfc3339")]
    pub ts: DateTime<Utc>,
    pub data: HashMap<String, LogValue>,
}

impl LogEntry {
    /// Filter log fields to only include defaults and those specified in `extra_fields`.
    ///
    /// This function modifies the log entry in place by removing fields that are not in the
    /// `ALWAYS_INCLUDE_KEYS` list or in `extra_fields`. This is useful for reducing the size of
    /// log entries sent to Memfault, as there are fields that are not useful or displayed.
    pub fn filter_fields(&mut self, extra_fields: &[String]) {
        self.data
            .retain(|k, _| ALWAYS_INCLUDE_KEYS.contains(&k.as_str()) || extra_fields.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use insta::{assert_json_snapshot, with_settings};
    use rstest::{fixture, rstest};

    use super::*;

    #[rstest]
    #[case("empty", "{}", "")]
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
        entry.filter_fields(&extra_attributes);

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(test_name, entry);
        });
    }

    #[fixture]
    fn time() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1334250000000).unwrap()
    }
}
