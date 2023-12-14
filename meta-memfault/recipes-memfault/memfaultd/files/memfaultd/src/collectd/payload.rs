//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use eyre::{eyre, Result};
use itertools::izip;
use log::warn;
use serde::Deserialize;

use crate::{
    metrics::{KeyedMetricReading, MetricReading, MetricStringKey},
    util::serialization::float_to_datetime,
    util::serialization::float_to_duration,
};

/// https://collectd.org/wiki/index.php/Data_source
/// and https://git.octo.it/?p=collectd.git;a=blob;f=src/daemon/plugin.h;hb=master#l45
#[derive(Debug, Deserialize)]
enum DataSourceType {
    #[serde(rename = "gauge")]
    Gauge,
    #[serde(rename = "derive")]
    Derive,
    #[serde(rename = "counter")]
    Counter,
    #[serde(rename = "absolute")]
    Absolute,
    #[serde(rename = "unknown")]
    Unknown,
}

/// https://collectd.org/wiki/index.php/JSON
/// https://collectd.org/wiki/index.php/Value_list
#[derive(Debug, Deserialize)]
pub struct Payload {
    dsnames: Vec<String>,
    dstypes: Vec<DataSourceType>,
    #[allow(dead_code)]
    host: String,
    // CollectD encodes time and duration to a float before sending as JSON.
    // https://github.com/collectd/collectd/blob/main/src/utils/format_json/format_json.c#L344-L345
    #[serde(with = "float_to_duration")]
    interval: Duration,
    plugin: String,
    plugin_instance: Option<String>,
    #[serde(with = "float_to_datetime")]
    time: DateTime<Utc>,
    #[serde(rename = "type")]
    type_str: String,
    type_instance: Option<String>,
    values: Vec<Option<f64>>,
}

impl Payload {
    fn metric_name(&self, name: &String) -> Result<MetricStringKey> {
        let use_simple_reading_name = self.dsnames.len() == 1 && self.dsnames[0] == "value";
        let name_prefix = vec![
            Some(&self.plugin),
            self.plugin_instance.as_ref(),
            Some(&self.type_str),
            self.type_instance.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter(|x| !x.is_empty())
        .map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("/");

        let name = if use_simple_reading_name {
            name_prefix
        } else {
            format!("{}/{}", name_prefix, name)
        };
        MetricStringKey::from_str(&name).map_err(|e| eyre!("Invalid metric name: {e}"))
    }
}

impl From<Payload> for Vec<KeyedMetricReading> {
    fn from(payload: Payload) -> Self {
        izip!(&payload.dsnames, &payload.values, &payload.dstypes)
            // Remove variables that have no value
            .filter_map(|(name, value, dstype)| value.as_ref().map(|v| (name, v, dstype)))
            // Remove variables with invalid names
            .filter_map(|(name, value, dstype)| match payload.metric_name(name) {
                Ok(key) => Some((key, value, dstype)),
                _ => {
                    warn!("Ignoring metric with invalid name: {}", name);
                    None
                }
            })
            // Create the KeyedMetricValue
            .map(|(key, value, dstype)| KeyedMetricReading {
                name: key,
                value: match dstype {
                    // Refer to https://github.com/collectd/collectd/wiki/Data-source
                    // for a general description of what CollectdD datasources are.

                    // Statsd generated counter values.
                    // See https://github.com/collectd/collectd/blob/7c5ce9f250aafbb6ef89769d7543ea155618b2ad/src/statsd.c#L799-L810
                    DataSourceType::Gauge if payload.type_str == "count" => {
                        MetricReading::Counter {
                            value: *value,
                            timestamp: payload.time,
                        }
                    }
                    DataSourceType::Gauge => MetricReading::Gauge {
                        value: *value,
                        timestamp: payload.time,
                        interval: payload.interval,
                    },
                    DataSourceType::Derive => MetricReading::Gauge {
                        value: *value,
                        timestamp: payload.time,
                        interval: payload.interval,
                    },
                    // A counter is a Derive (rate) that will never be negative.
                    DataSourceType::Counter => MetricReading::Gauge {
                        value: *value,
                        timestamp: payload.time,
                        interval: payload.interval,
                    },
                    DataSourceType::Absolute => MetricReading::Gauge {
                        value: *value,
                        timestamp: payload.time,
                        interval: payload.interval,
                    },
                    DataSourceType::Unknown => MetricReading::Gauge {
                        value: *value,
                        timestamp: payload.time,
                        interval: payload.interval,
                    },
                },
            })
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use std::fs::read_to_string;
    use std::path::PathBuf;

    use rstest::rstest;

    use crate::metrics::KeyedMetricReading;

    use super::DataSourceType;
    use super::Payload;

    #[rstest]
    #[case("A", Some("B"), "C", Some("D"), "E", "A/B/C/D/E")]
    #[case("A", None, "C", Some("D"), "E", "A/C/D/E")]
    #[case("A", Some("B"), "C", None, "E", "A/B/C/E")]
    #[case("A", None, "C", None, "E", "A/C/E")]
    #[case("A", Some(""), "C", Some(""), "E", "A/C/E")]
    #[case("A", Some("B"), "C", Some("D"), "value", "A/B/C/D")]
    fn convert_collectd_to_metric_name(
        #[case] plugin: &str,
        #[case] plugin_instance: Option<&str>,
        #[case] type_s: &str,
        #[case] type_instance: Option<&str>,
        #[case] dsname: &str,
        #[case] expected: &str,
    ) {
        let p = Payload {
            dsnames: vec![dsname.to_string()],
            dstypes: vec![DataSourceType::Gauge],
            host: "".to_string(),
            interval: Duration::seconds(10),
            plugin: plugin.to_string(),
            plugin_instance: plugin_instance.map(|x| x.to_owned()),
            time: chrono::Utc::now(),
            type_str: type_s.to_string(),
            type_instance: type_instance.map(|x| x.to_owned()),
            values: vec![Some(42.0)],
        };

        let readings = Vec::<KeyedMetricReading>::from(p);
        assert_eq!(readings.len(), 1);
        let KeyedMetricReading { name, .. } = &readings[0];
        assert_eq!(name.as_str(), expected)
    }

    #[rstest]
    // Note: sample1 contains multiple payloads. Some have equal timestamps (and need to be consolidated), some have "simple values".
    #[case("sample1")]
    #[case("sample-with-null")]
    #[case("statsd-counter-first-seen")]
    #[case("statsd-counter")]
    fn convert_collectd_payload_into_heartbeat_metadata(#[case] name: &str) {
        let input_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/collectd/fixtures")
            .join(name)
            .with_extension("json");

        // Read multiple payloads from a single file (this is how we receive from CollectD)
        let payloads =
            serde_json::from_str::<Vec<super::Payload>>(&read_to_string(&input_path).unwrap())
                .unwrap();

        // Convert payload into metric-readings
        let metadatas = payloads
            .into_iter()
            .flat_map(Vec::<KeyedMetricReading>::from)
            .collect::<Vec<_>>();

        // Check results
        insta::assert_json_snapshot!(format!("{name}"), metadatas);
    }
}
