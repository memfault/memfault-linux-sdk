//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(time: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let datetime_str = time.map(|t| t.to_rfc3339());
    match datetime_str {
        Some(datetime_str) => serializer.serialize_some(&datetime_str),
        None => serializer.serialize_none(),
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let datetime_str = <String>::deserialize(deserializer);
    match datetime_str {
        Ok(datetime_str) => {
            let datetime = DateTime::parse_from_rfc3339(&datetime_str)
                .ok()
                .map(|datetime| datetime.with_timezone(&Utc));
            Ok(datetime)
        }
        Err(_) => Ok(None),
    }
}
