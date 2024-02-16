//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::Read,
    path::PathBuf,
    str::{from_utf8, FromStr},
    sync::{Arc, Mutex},
};

use eyre::Result;
use tiny_http::{Method, Request, Response};

use crate::{
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::{MetricReportManager, MetricReportType, SessionName},
    network::NetworkConfig,
};

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

    fn parse_request(stream: &mut dyn Read) -> Result<SessionName> {
        let mut buf = vec![];
        stream.read_to_end(&mut buf)?;
        let reading = SessionName::from_str(from_utf8(&buf)?)?;
        Ok(reading)
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
                Ok(session_name) => {
                    if request.url() == "/v1/session/start" {
                        let mut metrics_store = self.metrics_store.lock().unwrap();
                        if let Err(e) = metrics_store.start_session(session_name) {
                            return HttpHandlerResult::Error(format!(
                                "Failed to start session: {:?}",
                                e
                            ));
                        }
                    } else if request.url() == "/v1/session/end" {
                        if let Err(e) = MetricReportManager::dump_report_to_mar_entry(
                            &self.metrics_store,
                            &self.mar_staging_path,
                            &self.network_config,
                            MetricReportType::Session(session_name),
                        ) {
                            return HttpHandlerResult::Error(format!(
                                "Failed to end session: {:?}",
                                e
                            ));
                        }
                    }
                }
                Err(e) => {
                    return HttpHandlerResult::Error(format!(
                        "Failed to parse session request: {:?}",
                        e
                    ))
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
        metrics::{MetricReportManager, MetricStringKey, SessionName},
        network::NetworkConfig,
        test_utils::in_gauges,
    };

    use super::SessionEventHandler;

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
        let readings = in_gauges(vec![
            ("foo", 1000, 1.0),
            ("bar", 1000, 2.0),
            ("baz", 1000, 3.0),
        ]);

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
    fn test_start_twice_without_stop_session(handler: SessionEventHandler) {
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
            let readings = in_gauges(vec![
                ("foo", 1000, 10.0),
                ("bar", 1000, 20.0),
                ("baz", 1000, 30.0),
            ]);

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
        let readings = in_gauges(vec![
            ("foo", 1000, 1.0),
            ("bar", 1000, 2.0),
            ("baz", 1000, 3.0),
        ]);

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
    fn test_start_then_stop_session(handler: SessionEventHandler) {
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
            .with_body("test-session");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));
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
}
