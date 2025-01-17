//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::Read;

use crate::http_server::{HttpHandler, HttpHandlerResult, MetricsRequest};
use eyre::Result;
use log::error;
use tiny_http::{Method, Request, Response};

use super::MetricsMBox;

pub struct MetricsEventHandler {
    metrics_mbox: MetricsMBox,
    data_collection_enabled: bool,
}

impl MetricsEventHandler {
    pub fn new(metrics_mbox: MetricsMBox, data_collection_enabled: bool) -> Self {
        Self {
            metrics_mbox,
            data_collection_enabled,
        }
    }

    pub fn parse_request<R: Read>(&self, reader: &mut R) -> Result<MetricsRequest> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        let request: MetricsRequest = serde_json::from_slice(&buf)?;
        Ok(request)
    }
}

impl HttpHandler for MetricsEventHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() != "/v1/metrics" || *request.method() != Method::Post {
            return HttpHandlerResult::NotHandled;
        }

        if self.data_collection_enabled {
            match self.parse_request(&mut request.as_reader()) {
                Ok(metrics_request) => {
                    match self.metrics_mbox.send_and_forget(metrics_request.readings) {
                        Ok(()) => HttpHandlerResult::Response(Response::empty(200).boxed()),
                        Err(e) => {
                            error!("Failed to send metrics to mailbox: {}", e);
                            HttpHandlerResult::Response(Response::empty(500).boxed())
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to parse metrics request: {}", e);
                    HttpHandlerResult::Response(Response::empty(400).boxed())
                }
            }
        } else {
            HttpHandlerResult::Response(Response::empty(200).boxed())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::metrics::{KeyedMetricReading, MetricReportManager, MetricStringKey, MetricValue};
    use crate::test_utils::setup_logger;

    use std::{collections::HashMap, io::Cursor};

    use insta::{assert_json_snapshot, with_settings};
    use rstest::{fixture, rstest};
    use ssf::ServiceJig;
    use tiny_http::TestRequest;

    const TEST_METRICS_BODY: &str = "
    {
        \"readings\": [
                {\"name\": \"Daisy\", \"value\": {\"Gauge\": {\"value\": 1.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}},
                {\"name\": \"Carlton\", \"value\": {\"Gauge\": {\"value\": 2.0, \"timestamp\": \"2024-01-01 00:00:00 UTC\"}}}
        ]
    }";

    #[rstest]
    fn test_parse_request(mut fixture: Fixture) {
        let handler = &mut fixture.handler;

        let key = MetricStringKey::from("Daisy");
        let reading = KeyedMetricReading::new_histogram(key.clone(), 42.0);
        let request = MetricsRequest {
            readings: vec![reading],
        };

        let request_json = serde_json::to_string(&request).unwrap();
        let mut cursor = Cursor::new(request_json.as_bytes());
        let parsed_request = handler.parse_request(&mut cursor).unwrap();

        assert_eq!(parsed_request.readings.len(), 1);
        assert_eq!(parsed_request.readings[0].name, key);
    }

    #[rstest]
    fn test_handle_request(mut fixture: Fixture, _setup_logger: ()) {
        let handler = &mut fixture.handler;

        let request = TestRequest::new()
            .with_path("/v1/metrics")
            .with_method(Method::Post)
            .with_body(TEST_METRICS_BODY);

        let response = handler.handle_request(&mut request.into());
        match response {
            HttpHandlerResult::Response(response) => {
                assert_eq!(response.status_code(), 200);
            }
            _ => panic!("Unexpected response"),
        }
        fixture.process_all();

        let metrics = fixture.take_metrics();
        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(metrics);
        });
    }

    #[rstest]
    fn test_handle_request_data_collection_disabled(mut fixture: Fixture, _setup_logger: ()) {
        fixture.set_data_collection_enabled(false);
        let handler = &mut fixture.handler;

        let request = TestRequest::new()
            .with_path("/v1/metrics")
            .with_method(Method::Post)
            .with_body(TEST_METRICS_BODY);

        let response = handler.handle_request(&mut request.into());
        match response {
            HttpHandlerResult::Response(response) => {
                assert_eq!(response.status_code(), 200);
            }
            _ => panic!("Unexpected response"),
        }
        fixture.process_all();

        let metrics = fixture.take_metrics();
        assert!(metrics.is_empty());
    }

    struct Fixture {
        handler: MetricsEventHandler,
        service: ServiceJig<MetricReportManager>,
    }

    impl Fixture {
        fn take_metrics(&mut self) -> HashMap<MetricStringKey, MetricValue> {
            self.service.get_service_mut().take_heartbeat_metrics()
        }

        fn process_all(&mut self) {
            self.service.process_all();
        }

        fn set_data_collection_enabled(&mut self, enabled: bool) {
            self.handler.data_collection_enabled = enabled;
        }
    }

    #[fixture]
    fn fixture() -> Fixture {
        let report_manager = MetricReportManager::default();
        let service = ServiceJig::prepare(report_manager);
        let handler = MetricsEventHandler::new(service.mailbox.clone().into(), true);

        Fixture { handler, service }
    }
}
