//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Contains the on disk representation of an HRT report.
//!
//! This module provides the schema for the on disk format of the high resolution telemetry data.
//! It also provides implementations to convert the in memory representation of the data to the on
//! disk format.

use std::io::Write;
use std::{fs::File, io::BufWriter, path::Path, time::Duration};

use chrono::Utc;
use eyre::{Error, Result};
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use serde_json::to_writer;

use super::report::HrtReport;
use crate::{
    build_info::VERSION,
    mar::{CompressionAlgorithm, MarEntry, MarEntryBuilder, Metadata},
    metrics::{KeyedMetricReading, MetricReading},
    network::NetworkConfig,
    util::{fs::DEFAULT_GZIP_COMPRESSION_LEVEL, serialization::milliseconds_to_duration},
};

const SCHEMA_VERSION: u32 = 1;
const PRODUCER_ID: &str = "memfaultd";
const MIME_TYPE: &str = "application/vnd.memfault.hrt.v1";
const FILE_NAME: &str = "hrt.json.gz";

#[derive(Serialize, Deserialize, Debug)]
/// Represents the on disk format of the high resolution telemetry data.
pub struct HighResTelemetryV1 {
    schema_version: u32,
    producer: Producer,
    start_time: i64,
    #[serde(rename = "duration_ms", with = "milliseconds_to_duration")]
    duration: Duration,
    rollups: Vec<Rollup>,
}

impl TryFrom<HrtReport> for HighResTelemetryV1 {
    type Error = Error;

    fn try_from(report: HrtReport) -> Result<Self, Self::Error> {
        let now = Utc::now();

        let rollups = report
            .readings
            .into_iter()
            .map(|(key, data)| Rollup {
                metadata: RollupMetadata {
                    string_key: key.to_string(),
                    metric_type: data.metadata.metric_type,
                    data_type: data.metadata.data_type,
                    internal: data.metadata.internal,
                },
                data: data.readings,
            })
            .collect();

        let duration = now - report.start_time;

        Ok(Self {
            schema_version: SCHEMA_VERSION,
            producer: Producer::new(),
            start_time: report.start_time.timestamp_millis(),
            duration: duration.to_std()?,
            rollups,
        })
    }
}

#[cfg(test)]
impl HighResTelemetryV1 {
    /// Helper function to easily sort rollups vector in snapshot tests
    pub fn sort_rollups(&mut self) {
        self.rollups
            .sort_by(|a, b| a.metadata.string_key.cmp(&b.metadata.string_key));
    }
}

#[derive(Serialize, Deserialize, Debug)]
/// A single HRT data point.
pub struct Datum {
    /// Timestamp in milliseconds since the epoch.
    t: i64,
    /// Value of the data point.
    value: f64,
}

impl From<&KeyedMetricReading> for Datum {
    fn from(reading: &KeyedMetricReading) -> Self {
        let (t, value) = match reading.value {
            MetricReading::TimeWeightedAverage {
                timestamp, value, ..
            } => (timestamp.timestamp_millis(), value),
            MetricReading::Histogram {
                timestamp, value, ..
            } => (timestamp.timestamp_millis(), value),
            MetricReading::Counter {
                timestamp, value, ..
            } => (timestamp.timestamp_millis(), value),
            MetricReading::Gauge {
                timestamp, value, ..
            } => (timestamp.timestamp_millis(), value),
            MetricReading::Rssi {
                timestamp, value, ..
            } => (timestamp.timestamp_millis(), value),
            // TODO: Add support for Strings in HRT
            MetricReading::ReportTag { timestamp, .. } => (timestamp.timestamp_millis(), 0.0),
            // TODO: Add support for Booleans in HRT
            MetricReading::Bool { timestamp, .. } => (timestamp.timestamp_millis(), 0.0),
        };
        Datum { t, value }
    }
}

#[derive(Serialize, Deserialize, Debug)]
/// Metadata for a single metric key.
pub struct RollupMetadata {
    string_key: String,
    metric_type: HrtMetricType,
    data_type: DataType,
    internal: bool,
}

#[derive(Serialize, Deserialize, Debug)]
/// Represents all data for a single metric.
pub struct Rollup {
    metadata: RollupMetadata,
    data: Vec<Datum>,
}

#[derive(Serialize, Deserialize, Debug)]
/// Tells us who produced the data.
pub struct Producer {
    id: String,
    version: String,
}

impl Producer {
    fn new() -> Self {
        Self {
            id: PRODUCER_ID.to_string(),
            version: VERSION.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
// TODO: Remove once used
#[allow(dead_code)]
/// Metric types as defined by the HRT schema.
pub enum HrtMetricType {
    Counter,
    Gauge,
    Property,
    Event,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
// TODO: Remove once used
#[allow(dead_code)]
/// Data types as defined by the HRT schema.
pub enum DataType {
    Double,
    String,
    Boolean,
}

/// Writes an HRT report to disk.
///
/// This function serializes the given HRT report to JSON and saves it in a CDR MAR entry.
pub fn write_report_to_disk(
    report: HrtReport,
    mar_path: &Path,
    network_config: &NetworkConfig,
) -> Result<MarEntry> {
    let start_time = report.start_time;
    let hrt = HighResTelemetryV1::try_from(report)?;

    let mar_builder =
        MarEntryBuilder::new(mar_path)?.set_metadata(Metadata::new_custom_data_recording(
            Some(start_time),
            hrt.duration,
            vec![MIME_TYPE.to_string()],
            "hrt".to_string(),
            FILE_NAME.to_string(),
            Some(CompressionAlgorithm::Gzip),
        ));

    let hrt_path = mar_builder.make_attachment_path_in_entry_dir(FILE_NAME);
    let hrt_file = File::create(hrt_path)?;
    let mut gz_encoder = BufWriter::new(GzEncoder::new(hrt_file, DEFAULT_GZIP_COMPRESSION_LEVEL));
    to_writer(&mut gz_encoder, &hrt)?;
    gz_encoder.flush()?;

    mar_builder.save(network_config)
}

#[cfg(test)]
mod test {
    use std::{fs::File, io::BufReader, num::NonZeroU32};

    use super::*;

    use crate::{
        mar::manifest,
        metrics::{
            hrt::{
                report::{HrtReadingData, HrtReadingMetadata},
                HrtReport,
            },
            MetricStringKey,
        },
        network::NetworkConfig,
        util::rate_limiter::RateLimiter,
    };

    use chrono::{TimeZone, Utc};
    use flate2::bufread::GzDecoder;
    use insta::assert_json_snapshot;
    use rstest::rstest;
    use serde_json::json;

    #[test]
    fn test_serialization() {
        let mut report = build_report();

        report.readings.insert(
            "test".into(),
            HrtReadingData {
                readings: vec![Datum {
                    t: 1010,
                    value: 220.0,
                }],
                metadata: HrtReadingMetadata {
                    metric_type: HrtMetricType::Counter,
                    data_type: DataType::Double,
                    internal: false,
                },
            },
        );

        let hrt = HighResTelemetryV1::try_from(report).unwrap();
        assert_json_snapshot!(json!(hrt), {".duration_ms" => 0, ".producer.version" => "1.2.3"});
    }

    #[test]
    fn test_cdr_write() {
        let mut report = build_report();

        report.readings.insert(
            "test".into(),
            HrtReadingData {
                readings: vec![
                    Datum {
                        t: 1234,
                        value: 567.0,
                    },
                    Datum {
                        t: 5678,
                        value: 123.0,
                    },
                ],
                metadata: HrtReadingMetadata {
                    metric_type: HrtMetricType::Counter,
                    data_type: DataType::Double,
                    internal: false,
                },
            },
        );

        let tmp_dir = tempfile::tempdir().unwrap();
        let mar_path = tmp_dir.path();

        let entry = write_report_to_disk(report, mar_path, &NetworkConfig::test_fixture()).unwrap();

        let hrt_filename = match entry.manifest.metadata {
            Metadata::CustomDataRecording {
                recording_file_name,
                ..
            } => recording_file_name,
            _ => panic!("Unexpected metadata type"),
        };

        let hrt_file_path = entry.path.join(hrt_filename);
        let manifest_path = entry.path.join("manifest.json");
        assert!(hrt_file_path.exists());
        assert!(manifest_path.exists());

        let hrt_file = File::open(hrt_file_path).unwrap();
        let gz_decoder = GzDecoder::new(BufReader::new(hrt_file));
        let hrt: HighResTelemetryV1 = serde_json::from_reader(gz_decoder).unwrap();
        assert_json_snapshot!(json!(hrt), {".duration_ms" => 0, ".producer.version" => "1.2.3"});

        let manifest_file = File::open(manifest_path).unwrap();
        let manifest: manifest::Manifest = serde_json::from_reader(manifest_file).unwrap();
        assert_json_snapshot!(manifest.metadata, {".metadata.duration_ms" => 0});
    }

    #[rstest]
    #[case("counter", MetricReading::Counter { value: 123.0, timestamp: Utc.timestamp_millis_opt(1234).unwrap() })]
    #[case("gauge", MetricReading::Gauge { value: 123.0, timestamp: Utc.timestamp_millis_opt(1234).unwrap() })]
    #[case("histogram", MetricReading::Histogram { value: 123.0, timestamp: Utc.timestamp_millis_opt(1234).unwrap() })]
    fn test_datum_conversion(#[case] case: &str, #[case] reading: MetricReading) {
        let metric = KeyedMetricReading {
            name: MetricStringKey::from("test"),
            value: reading,
        };
        let datum = Datum::from(&metric);

        assert_json_snapshot!(case, json!(datum));
    }

    fn build_report() -> HrtReport {
        let start_time = Utc.with_ymd_and_hms(1991, 3, 25, 0, 0, 0).unwrap();
        HrtReport {
            start_time,
            readings: Default::default(),
            rate_limiter: RateLimiter::new(
                NonZeroU32::new(7500).expect("Zero value passed to non-zero constructor"),
            ),
        }
    }
}
