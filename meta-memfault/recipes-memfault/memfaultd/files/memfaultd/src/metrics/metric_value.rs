//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Number(f64),
    String(String),
}

impl Serialize for MetricValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MetricValue::Number(v) => serializer.serialize_f64(*v),
            MetricValue::String(v) => serializer.serialize_str(v.as_str()),
        }
    }
}
