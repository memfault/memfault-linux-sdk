//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::Read,
    path::PathBuf,
    str::{from_utf8, FromStr},
};

use eyre::{eyre, Result};
use log::warn;
use ssf::MsgMailbox;
use tiny_http::{Method, Request, Response};

use crate::{
    http_server::{HttpHandler, HttpHandlerResult, SessionRequest},
    metrics::SessionName,
    network::NetworkConfig,
};

use super::SessionEventMessage;

/// A server that listens for session management requests
#[derive(Clone)]
pub struct SessionEventHandler {
    data_collection_enabled: bool,
    session_events_mbox: MsgMailbox<SessionEventMessage>,
    mar_staging_path: PathBuf,
    network_config: NetworkConfig,
}

impl SessionEventHandler {
    pub fn new(
        data_collection_enabled: bool,
        session_events_mbox: MsgMailbox<SessionEventMessage>,
        mar_staging_path: PathBuf,
        network_config: NetworkConfig,
    ) -> Self {
        Self {
            data_collection_enabled,
            session_events_mbox,
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

    fn start_session(
        &self,
        name: SessionName,
        readings: Vec<super::KeyedMetricReading>,
    ) -> Result<()> {
        self.session_events_mbox
            .send_and_wait_for_reply(SessionEventMessage::StartSession { name, readings })?
    }

    fn stop_session(
        &self,
        name: SessionName,
        readings: Vec<super::KeyedMetricReading>,
    ) -> Result<()> {
        self.session_events_mbox
            .send_and_wait_for_reply(SessionEventMessage::StopSession {
                name,
                readings,
                network_config: self.network_config.clone(),
                mar_staging_area: self.mar_staging_path.clone(),
            })?
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
                    readings,
                }) => {
                    if request.url() == "/v1/session/start" {
                        if let Err(e) = self.start_session(session_name, readings) {
                            return HttpHandlerResult::Error(format!(
                                "Failed to start session: {:?}",
                                e
                            ));
                        }
                    } else if request.url() == "/v1/session/end" {
                        if let Err(e) = self.stop_session(session_name, readings) {
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
        collections::{BTreeMap, HashSet},
        path::Path,
        str::FromStr,
    };

    use insta::assert_json_snapshot;
    use rstest::{fixture, rstest};
    use ssf::{PingMessage, SharedServiceThread};
    use tempfile::TempDir;
    use tiny_http::{Method, TestRequest};

    use crate::{
        config::SessionConfig,
        http_server::{HttpHandler, HttpHandlerResult},
        mar::manifest::{Manifest, Metadata},
        metrics::{
            KeyedMetricReading, MetricReportManager, MetricStringKey, MetricValue, SessionName,
        },
        test_utils::in_histograms,
    };

    use super::*;
    use crate::test_utils::setup_logger;

    #[rstest]
    fn test_start_without_stop_session(mut fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        fixture.send_metrics(&mut in_histograms(vec![
            ("foo", 1.0),
            ("bar", 2.0),
            ("not-captured", 3.0),
        ]));

        assert_json_snapshot!(fixture.take_session_metrics());
    }

    #[rstest]
    fn test_start_with_metrics(_setup_logger: (), mut fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("{\"session_name\": \"test-session\",
                         \"readings\":
                                [
                                  {\"name\": \"foo\", \"value\": {\"Gauge\": {\"value\": 1.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                                  {\"name\": \"bar\", \"value\": {\"Gauge\": {\"value\": 4.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                                  {\"name\": \"baz\", \"value\": {\"ReportTag\": {\"value\": \"test-tag\", \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
                                ]
                         }");
        let response = fixture.handler.handle_request(&mut r.into());
        assert!(matches!(response, HttpHandlerResult::Response(_)));

        assert_json_snapshot!(fixture.take_session_metrics());
    }

    #[rstest]
    fn test_end_with_metrics(_setup_logger: (), fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/end")
            .with_body("{\"session_name\": \"test-session\",
                         \"readings\":
                                [
                                  {\"name\": \"foo\", \"value\": {\"Gauge\": {\"value\": 1.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                                  {\"name\": \"bar\", \"value\": {\"Gauge\": {\"value\": 3.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
                                ]
                         }");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));
        fixture.process_all();

        verify_dumped_metric_report(fixture.tempdir.path(), "end_with_metrics")
    }

    #[rstest]
    fn test_start_twice_without_stop_session(_setup_logger: (), mut fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");

        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        fixture.send_metrics(&mut in_histograms(vec![
            ("foo", 10.0),
            ("bar", 20.0),
            ("not-captured", 30.0),
        ]));

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("test-session");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        fixture.send_metrics(&mut in_histograms(vec![
            ("foo", 1.0),
            ("bar", 2.0),
            ("not-captured", 3.0),
        ]));

        assert_json_snapshot!(fixture.take_session_metrics());
    }

    #[rstest]
    fn test_start_then_stop_session(_setup_logger: (), mut fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/start")
            .with_body("{\"session_name\": \"test-session\", \"readings\": []}");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));
        fixture.send_metrics(&mut in_histograms(vec![
            ("bar", 20.0),
            ("not-captured", 30.0),
        ]));

        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/end")
            .with_body("{\"session_name\": \"test-session\",
                         \"readings\":
                                [
                                  {\"name\": \"foo\", \"value\": {\"Gauge\": {\"value\": 100, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                                  {\"name\": \"baz\", \"value\": {\"ReportTag\": {\"value\": \"test-tag\", \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
                                ]
                         }");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));
        fixture.process_all();

        verify_dumped_metric_report(fixture.tempdir.path(), "start_then_stop");

        // Should error as session should have been removed from MetricReportManager
        // after it was ended
        assert!(fixture
            .jig
            .shared()
            .lock()
            .unwrap()
            .take_session_metrics(&SessionName::from_str("test-session").unwrap())
            .is_err());
    }

    #[rstest]
    fn test_stop_without_start_session(_setup_logger: (), fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/session/end")
            .with_body("test-session");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Error(_)
        ));
    }

    struct Fixture {
        handler: SessionEventHandler,
        jig: SharedServiceThread<MetricReportManager>,
        tempdir: TempDir,
    }

    impl Fixture {
        fn take_session_metrics(&mut self) -> BTreeMap<MetricStringKey, MetricValue> {
            self.process_all();
            self.jig
                .shared()
                .lock()
                .unwrap()
                .take_session_metrics(&SessionName::from_str("test-session").unwrap())
                .unwrap()
                .into_iter()
                .collect()
        }

        fn send_metrics(&mut self, metrics: &mut dyn Iterator<Item = KeyedMetricReading>) {
            self.jig
                .mbox()
                .send_and_wait_for_reply(metrics.collect::<Vec<_>>())
                .expect("error delivering metrics");
        }

        fn process_all(&self) {
            self.jig
                .mbox()
                .send_and_wait_for_reply(PingMessage {})
                .expect("unable to ping thread")
        }
    }

    /// Creates a SessionEventHandler whose metric store is configured with
    /// a "test-session" session that captures the "foo" and "bar" metrics
    #[fixture]
    fn fixture() -> Fixture {
        let session_config = SessionConfig {
            name: SessionName::from_str("test-session").unwrap(),
            captured_metrics: HashSet::from_iter([
                MetricStringKey::from_str("foo").unwrap(),
                MetricStringKey::from_str("bar").unwrap(),
                MetricStringKey::from_str("baz").unwrap(),
            ]),
        };

        let jig =
            SharedServiceThread::spawn_with(MetricReportManager::new_with_session_configs(&[
                session_config,
            ]));

        let tempdir = TempDir::new().unwrap();
        let handler = SessionEventHandler::new(
            true,
            jig.mbox().into(),
            tempdir.path().to_owned(),
            NetworkConfig::test_fixture(),
        );
        Fixture {
            handler,
            jig,
            tempdir,
        }
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
