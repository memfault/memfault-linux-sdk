//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde::{Deserialize, Deserializer, Serializer};

use std::time::Duration;

pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(duration) = duration {
        serializer.serialize_u128(duration.as_millis())
    } else {
        serializer.serialize_none()
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    if let Ok(secs) = u64::deserialize(deserializer) {
        Ok(Some(Duration::from_millis(secs)))
    } else {
        Ok(None)
    }
}
