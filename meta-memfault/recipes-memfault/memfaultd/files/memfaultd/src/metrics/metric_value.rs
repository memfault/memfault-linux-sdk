//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use serde::{Deserialize, Serialize, Serializer};
use std::fmt::Display;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Histogram {
    pub min: f64,
    pub mean: f64,
    pub max: f64,
}

impl Histogram {
    pub fn min(&self) -> MetricValue {
        MetricValue::Number(self.min)
    }

    pub fn avg(&self) -> MetricValue {
        MetricValue::Number(self.mean)
    }

    pub fn max(&self) -> MetricValue {
        MetricValue::Number(self.max)
    }
}

pub fn construct_histogram_value(min: f64, mean: f64, max: f64) -> Histogram {
    Histogram { min, mean, max }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Number(f64),
    String(String),
    Histogram(Histogram),
    Bool(bool),
}

impl Serialize for MetricValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MetricValue::Number(v) => serializer.serialize_f64(*v),
            MetricValue::String(v) => serializer.serialize_str(v.as_str()),
            MetricValue::Histogram(histo) => histo.serialize(serializer),
            MetricValue::Bool(v) => serializer.serialize_bool(*v),
        }
    }
}

impl Display for MetricValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let json_value = match self {
            MetricValue::Number(n) => serde_json::Value::Number(
                serde_json::Number::from_f64(*n).unwrap_or(serde_json::Number::from(0)),
            ),
            MetricValue::String(s) => serde_json::Value::String(s.clone()),
            MetricValue::Bool(b) => serde_json::Value::Bool(*b),
            MetricValue::Histogram(h) => {
                // For histograms, show basic stats
                serde_json::json!({
                    "type": "histogram",
                    "avg": h.mean,
                    "min": h.min,
                    "max": h.max
                })
            }
        };
        write!(f, "{}", json_value)
    }
}
