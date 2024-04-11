//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::Read,
    path::PathBuf,
    str::{from_utf8, FromStr},
    sync::{Arc, Mutex},
};

use eyre::{eyre, Result};
use log::warn;
use tiny_http::{Method, Request, Response};

use crate::{
    http_server::{HttpHandler, HttpHandlerResult, SessionRequest},
    metrics::{MetricReportManager, MetricReportType, SessionName},
    network::NetworkConfig,
};

use super::KeyedMetricReading;

/// A server that listens for session management requests
#[derive(Clone)]
pub struct SessionEventHandler {
    data_collection_enabled: bool,
    metrics_store: Arc<Mutex<MetricReportManager>>,
    mar_staging_path: PathBuf,
    network_config: NetworkConfig,
}

impl SessionEventHandler {
    pub fn new(
        data_collection_enabled: bool,
        metrics_store: Arc<Mutex<MetricReportManager>>,
        mar_staging_path: PathBuf,
        network_config: NetworkConfig,
    ) -> Self {
        Self {
            data_collection_enabled,
            metrics_store,
            mar_staging_path,
            network_config,
        }
    }

    fn parse_request(stream: &mut dyn Read) -> Result<SessionRequest> {
        let mut buf = vec![];
        stream.read_to_end(&mut buf)?;
        let body = from_utf8(&buf)?;
        match serde_json::from_str(body) {
            Ok(request_body) => Ok(request_body),
            // Fall back to legacy API, SessionName as raw str in body (no JSON)
            Err(e) => {
                // If the request doesn't match either the JSON API or legacy API,
                // include JSON API parse error in response (as that is currently
                // the standard API)
                Ok(SessionRequest::new_without_readings(
                    SessionName::from_str(body)
                        .map_err(|_| eyre!("Couldn't parse request: {}", e))?,
                ))
            }
        }
    }

    fn add_metric_readings_to_session(
        session_name: &SessionName,
        metric_reports: Arc<Mutex<MetricReportManager>>,
        metric_readings: Vec<KeyedMetricReading>,
    ) -> Result<()> {
        let mut metric_reports = metric_reports.lock().expect("Mutex poisoned!");
        for metric_reading in metric_readings {
            metric_reports.add_metric_to_report(
                &MetricReportType::Session(session_name.clone()),
                metric_reading,
            )?
        }

        Ok(())
    }
}

impl HttpHandler for SessionEventHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if (request.url() != "/v1/session/start" && request.url() != "/v1/session/end")
            || *request.method() != Method::Post
        {
            return HttpHandlerResult::NotHandled;
        }

        if self.data_collection_enabled {
            match Self::parse_request(request.as_reader()) {
                Ok(SessionRequest {
                    session_name,
                    gauge_readings,
                }) => {
                    if request.url() == "/v1/session/start" {
                        if let Err(e) = self
                            .metrics_store
                            .lock()
                            .expect("Mutex poisoned")
                            .start_session(session_name.clone())
                        {
                            return HttpHandlerResult::Error(format!(
                                "Failed to start session: {:?}",
                                e
                            ));
                        }
                    }

                    // Add additional metric readings after the session has started
                    // (if starting) but before it has ended (if ending)
                    if !gauge_readings.is_empty() {
                        if let Err(e) = Self::add_metric_readings_to_session(
                            &session_name,
                            self.metrics_store.clone(),
                            gauge_readings,
                        ) {
                            warn!("Failed to add metrics to session report: {}", e);
                        }
                    }

                    if request.url() == "/v1/session/end" {
                        if let Err(e) = MetricReportManager::dump_report_to_mar_entry(
                            &self.metrics_store,
                            &self.mar_staging_path,
                            &self.network_config,
                            &MetricReportType::Session(session_name),
                        ) {
                            return HttpHandlerResult::Error(format!(
                                "Failed to end session: {:?}",
                                e
                            ));
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to parse session request: {:?}", e);
                    return HttpHandlerResult::Error(format!(
                        "Failed to parse session request: {:?}",
                        e
                    ));
                }
            }
        }
        HttpHandlerResult::Response(Response::empty(200).boxed())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::Path,
        str::FromStr,
        sync::{Arc, Mutex},
    };

    use insta::assert_json_snapshot;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;
    use tiny_http::{Method, TestRequest};

    use crate::{
        config::SessionConfig,
        http_server::{HttpHandler, HttpHandlerResult},
        mar::manifest::{Manifest, Metadata},
        metrics::{MetricReportManager, MetricStringKey, SessionName},
        network::NetworkConfig,
        test_utils::in_histograms,
    };

    use super::*;
    use crate::test_utils::setup_logger;

    #[rstest]
    fn test_start_without_stop_session(handler: SessionEventHandler) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let mut metric_report_manager = handler.metrics_store.lock().unwrap();
        let readings = in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz", 3.0)]);

        for reading in readings {
            metric_report_manager
                .add_metric(reading)
                .expect("Failed to add metric reading");
        }

        let metrics: BTreeMap<_, _> = metric_report_manager
            .take_session_metrics(&SessionName::from_str("test-session").unwrap())
            .unwrap()
            .into_iter()
            .collect();

        assert_json_snapshot!(metrics);
    }

    #[rstest]
    fn test_start_with_metrics(_setup_logger: (), handler: SessionEventHandler) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("{\"session_name\": \"test-session\", 
                         \"gauge_readings\": 
                                [ 
                                  {\"name\": \"foo\", \"value\": {\"Gauge\": {\"value\": 1.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                                  {\"name\": \"bar\", \"value\": {\"Gauge\": {\"value\": 4.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
                                ]
                         }");
        let response = handler.handle_request(&mut r.into());
        assert!(matches!(response, HttpHandlerResult::Response(_)));

        let mut metric_report_manager = handler.metrics_store.lock().unwrap();
        let metrics: BTreeMap<_, _> = metric_report_manager
            .take_session_metrics(&SessionName::from_str("test-session").unwrap())
            .unwrap()
            .into_iter()
            .collect();

        assert_json_snapshot!(metrics);
    }

    #[rstest]
    fn test_end_with_metrics(_setup_logger: ()) {
        let session_config = SessionConfig {
            name: SessionName::from_str("test-session").unwrap(),
            captured_metrics: vec![
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
            ],
        };

        let tempdir = TempDir::new().unwrap();
        let handler = SessionEventHandler::new(
            true,
            Arc::new(Mutex::new(MetricReportManager::new_with_session_configs(
                &[session_config],
            ))),
            tempdir.path().to_path_buf(),
            NetworkConfig::test_fixture(),
        );
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/end")
            .with_body("{\"session_name\": \"test-session\", 
                         \"gauge_readings\": 
                                [ 
                                  {\"name\": \"foo\", \"value\": {\"Gauge\": {\"value\": 1.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                                  {\"name\": \"bar\", \"value\": {\"Gauge\": {\"value\": 3.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
                                ]
                         }");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        verify_dumped_metric_report(&handler.mar_staging_path, "end_with_metrics")
    }

    #[rstest]
    fn test_start_twice_without_stop_session(_setup_logger: (), handler: SessionEventHandler) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");

        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        {
            let mut metric_report_manager = handler.metrics_store.lock().unwrap();
            let readings = in_histograms(vec![("foo", 10.0), ("bar", 20.0), ("baz", 30.0)]);

            for reading in readings {
                metric_report_manager
                    .add_metric(reading)
                    .expect("Failed to add metric reading");
            }
        }

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let mut metric_report_manager = handler.metrics_store.lock().unwrap();
        let readings = in_histograms(vec![("foo", 1.0), ("bar", 2.0), ("baz", 3.0)]);

        for reading in readings {
            metric_report_manager
                .add_metric(reading)
                .expect("Failed to add metric reading");
        }

        let metrics: BTreeMap<_, _> = metric_report_manager
            .take_session_metrics(&SessionName::from_str("test-session").unwrap())
            .unwrap()
            .into_iter()
            .collect();

        assert_json_snapshot!(metrics);
    }

    #[rstest]
    fn test_start_then_stop_session(_setup_logger: ()) {
        let session_config = SessionConfig {
            name: SessionName::from_str("test-session").unwrap(),
            captured_metrics: vec![
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
            ],
        };

        let tempdir = TempDir::new().unwrap();
        let handler = SessionEventHandler::new(
            true,
            Arc::new(Mutex::new(MetricReportManager::new_with_session_configs(
                &[session_config],
            ))),
            tempdir.path().to_path_buf(),
            NetworkConfig::test_fixture(),
        );

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("{\"session_name\": \"test-session\", \"gauge_readings\": []}");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));
        {
            let mut metric_report_manager = handler.metrics_store.lock().unwrap();
            let readings = in_histograms(vec![("bar", 20.0), ("baz", 30.0)]);

            for reading in readings {
                metric_report_manager
                    .add_metric(reading)
                    .expect("Failed to add metric reading");
            }
        }

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/end")
            .with_body("{\"session_name\": \"test-session\", 
                         \"gauge_readings\": 
                                [ 
                                  {\"name\": \"foo\", \"value\": {\"Gauge\": {\"value\": 100, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
                                ]
                         }");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        verify_dumped_metric_report(&handler.mar_staging_path, "start_then_stop");
        let mut metric_report_manager = handler.metrics_store.lock().unwrap();

        // Should error as session should have been removed from MetricReportManager
        // after it was ended
        assert!(metric_report_manager
            .take_session_metrics(&SessionName::from_str("test-session").unwrap())
            .is_err());
    }

    #[rstest]
    fn test_stop_without_start_session(handler: SessionEventHandler) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/end")
            .with_body("test-session");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Error(_)
        ));
    }

    /// Creates a SessionEventHandler whose metric store is configured with
    /// a "test-session" session that captures the "foo" and "bar" metrics
    #[fixture]
    fn handler() -> SessionEventHandler {
        let session_config = SessionConfig {
            name: SessionName::from_str("test-session").unwrap(),
            captured_metrics: vec![
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
            ],
        };

        SessionEventHandler::new(
            true,
            Arc::new(Mutex::new(MetricReportManager::new_with_session_configs(
                &[session_config],
            ))),
            TempDir::new().unwrap().path().to_path_buf(),
            NetworkConfig::test_fixture(),
        )
    }

    fn verify_dumped_metric_report(mar_staging_path: &Path, test_name: &str) {
        let mar_dir = std::fs::read_dir(mar_staging_path)
            .expect("Failed to read temp dir")
            .filter_map(|entry| entry.ok())
            .collect::<Vec<_>>();

        // There should only be an entry for the reboot reason
        assert_eq!(mar_dir.len(), 1);

        let mar_manifest = mar_dir[0].path().join("manifest.json");
        let manifest_string = std::fs::read_to_string(mar_manifest).unwrap();
        let manifest: Manifest = serde_json::from_str(&manifest_string).unwrap();

        if let Metadata::LinuxMetricReport { .. } = manifest.metadata {
            assert_json_snapshot!(test_name, manifest.metadata);
        } else {
            panic!("Unexpected metadata type");
        }
    }
}
