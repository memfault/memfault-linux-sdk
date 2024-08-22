//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use log::warn;
use tiny_http::{Method, Request, Response};

use crate::{
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::{
        core_metrics::{METRIC_SYNC_FAILURE, METRIC_SYNC_SUCCESS},
        KeyedMetricReading, MetricsMBox,
    },
};

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct ReportSyncEventHandler {
    data_collection_enabled: bool,
    metrics_mbox: MetricsMBox,
}

impl ReportSyncEventHandler {
    pub fn new(data_collection_enabled: bool, metrics_mbox: MetricsMBox) -> Self {
        Self {
            data_collection_enabled,
            metrics_mbox,
        }
    }
}

impl HttpHandler for ReportSyncEventHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if (request.url() != "/v1/sync/success" && request.url() != "/v1/sync/failure")
            || *request.method() != Method::Post
        {
            return HttpHandlerResult::NotHandled;
        }
        if self.data_collection_enabled {
            if request.url() == "/v1/sync/success" {
                if let Err(e) =
                    self.metrics_mbox
                        .send_and_forget(vec![KeyedMetricReading::increment_counter(
                            METRIC_SYNC_SUCCESS.into(),
                        )])
                {
                    warn!("Couldn't increment sync_success counter: {:#}", e);
                }
            } else if request.url() == "/v1/sync/failure" {
                if let Err(e) =
                    self.metrics_mbox
                        .send_and_forget(vec![KeyedMetricReading::increment_counter(
                            METRIC_SYNC_FAILURE.into(),
                        )])
                {
                    warn!("Couldn't increment sync_failure counter: {:#}", e);
                }
            }
        }
        HttpHandlerResult::Response(Response::empty(200).boxed())
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_json_snapshot;
    use rstest::{fixture, rstest};
    use ssf::ServiceMock;
    use tiny_http::{Method, TestRequest};

    use crate::{
        http_server::{HttpHandler, HttpHandlerResult},
        metrics::{KeyedMetricReading, TakeMetrics},
    };

    use super::ReportSyncEventHandler;

    #[rstest]
    fn handle_sync_success(mut fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/sync/success");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        assert_json_snapshot!(fixture.mock.take_metrics().unwrap());
    }

    #[rstest]
    fn handle_sync_failure(mut fixture: Fixture) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/sync/failure");
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));
        assert_json_snapshot!(fixture.mock.take_metrics().unwrap());
    }

    #[rstest]
    fn handle_multiple_sync_events(mut fixture: Fixture) {
        for _ in 0..10 {
            let r = TestRequest::new()
                .with_method(Method::Post)
                .with_path("/v1/sync/failure");
            assert!(matches!(
                fixture.handler.handle_request(&mut r.into()),
                HttpHandlerResult::Response(_)
            ));
        }

        for _ in 0..90 {
            let r = TestRequest::new()
                .with_method(Method::Post)
                .with_path("/v1/sync/success");
            assert!(matches!(
                fixture.handler.handle_request(&mut r.into()),
                HttpHandlerResult::Response(_)
            ));
        }
        assert_json_snapshot!(fixture.mock.take_metrics().unwrap());
    }

    struct Fixture {
        handler: ReportSyncEventHandler,
        mock: ServiceMock<Vec<KeyedMetricReading>>,
    }
    #[fixture]
    fn fixture() -> Fixture {
        let mock = ServiceMock::new();
        let handler = ReportSyncEventHandler::new(true, mock.mbox.clone());

        Fixture { handler, mock }
    }
}
