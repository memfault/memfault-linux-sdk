//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(time: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_f64(
        time.timestamp() as f64 + (time.timestamp_subsec_micros() as f64 / 1_000_000.0),
    )
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let secs = f64::deserialize(deserializer)?;

    // Collectd only sends milli-seconds. We round the float to the nearest ms
    // to avoid precision error.
    let ms = ((secs.rem_euclid(1.0)) * 1e3).round() as u32;

    match NaiveDateTime::from_timestamp_opt(secs.floor() as i64, ms * 1_000_000) {
        Some(naive) => Ok(DateTime::<Utc>::from_utc(naive, Utc)),
        None => Err(serde::de::Error::custom("invalid timestamp")),
    }
}
