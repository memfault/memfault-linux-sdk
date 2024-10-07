//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use serde::{Deserialize, Serialize, Serializer};

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
        }
    }
}
