//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Duration;
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer
        .serialize_f64(duration.num_seconds() as f64 + duration.num_milliseconds() as f64 / 1000.0)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let d = f64::deserialize(deserializer)?;
    let seconds = d.trunc() as i64;
    let ms = (d.rem_euclid(1.0) * 1000.0) as i64;
    Ok(Duration::seconds(seconds) + Duration::milliseconds(ms))
}
