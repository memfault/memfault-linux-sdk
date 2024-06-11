//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(time: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let datetime_str = time.to_rfc3339();
    serializer.serialize_str(&datetime_str)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let datetime_str = String::deserialize(deserializer)?;
    let datetime = DateTime::parse_from_rfc3339(&datetime_str)
        .map_err(|e| serde::de::Error::custom(format!("invalid timestamp: {}", e)))?;

    Ok(datetime.with_timezone(&Utc))
}
