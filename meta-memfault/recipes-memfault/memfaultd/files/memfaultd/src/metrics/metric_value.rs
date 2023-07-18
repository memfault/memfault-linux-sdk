//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Number(f64),
}

impl Serialize for MetricValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MetricValue::Number(v) => serializer.serialize_f64(*v),
        }
    }
}
