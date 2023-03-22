//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use flate2::Compression;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S>(compression: &Compression, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    (compression.level()).serialize(serializer)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Compression, D::Error>
where
    D: Deserializer<'de>,
{
    let level = u32::deserialize(deserializer)?;
    match level {
        0..=9 => Ok(Compression::new(level)),
        _ => Err(serde::de::Error::custom(
            "Compression level must be between 0 and 9.",
        )),
    }
}
