//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::{Arc, Mutex};

use log::warn;
use tiny_http::{Method, Request, Response};

use crate::{
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::MetricReportManager,
};

const METRIC_SYNC_SUCCESS: &str = "sync_successful";
const METRIC_SYNC_FAILURE: &str = "sync_failure";

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct ReportSyncEventHandler {
    data_collection_enabled: bool,
    metrics_store: Arc<Mutex<MetricReportManager>>,
}

impl ReportSyncEventHandler {
    pub fn new(
        data_collection_enabled: bool,
        metrics_store: Arc<Mutex<MetricReportManager>>,
    ) -> Self {
        Self {
            data_collection_enabled,
            metrics_store,
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
                let mut metrics_store = self.metrics_store.lock().unwrap();
                if let Err(e) = metrics_store.increment_counter(METRIC_SYNC_SUCCESS) {
                    warn!("Couldn't increment sync_success counter: {:#}", e);
                }
            } else if request.url() == "/v1/sync/failure" {
                let mut metrics_store = self.metrics_store.lock().unwrap();
                if let Err(e) = metrics_store.increment_counter(METRIC_SYNC_FAILURE) {
                    warn!("Couldn't increment sync_failure counter: {:#}", e);
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
        sync::{Arc, Mutex},
    };

    use insta::assert_json_snapshot;
    use rstest::{fixture, rstest};
    use tiny_http::{Method, TestRequest};

    use crate::{
        http_server::{HttpHandler, HttpHandlerResult},
        metrics::MetricReportManager,
    };

    use super::ReportSyncEventHandler;

    #[rstest]
    fn handle_sync_success(handler: ReportSyncEventHandler) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/sync/success");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let metrics = handler
            .metrics_store
            .lock()
            .unwrap()
            .take_heartbeat_metrics();
        assert_json_snapshot!(&metrics);
    }

    #[rstest]
    fn handle_sync_failure(handler: ReportSyncEventHandler) {
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/sync/failure");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let metrics = handler
            .metrics_store
            .lock()
            .unwrap()
            .take_heartbeat_metrics();
        assert_json_snapshot!(&metrics);
    }

    #[rstest]
    fn handle_multiple_sync_events(handler: ReportSyncEventHandler) {
        for _ in 0..10 {
            let r = TestRequest::new()
                .with_method(Method::Post)
                .with_path("/v1/sync/failure");
            assert!(matches!(
                handler.handle_request(&mut r.into()),
                HttpHandlerResult::Response(_)
            ));
        }

        for _ in 0..90 {
            let r = TestRequest::new()
                .with_method(Method::Post)
                .with_path("/v1/sync/success");
            assert!(matches!(
                handler.handle_request(&mut r.into()),
                HttpHandlerResult::Response(_)
            ));
        }
        let metrics = handler
            .metrics_store
            .lock()
            .unwrap()
            .take_heartbeat_metrics();
        // Need to sort the map so the JSON string is consistent
        let sorted_metrics: BTreeMap<_, _> = metrics.iter().collect();

        assert_json_snapshot!(&sorted_metrics);
    }

    #[fixture]
    fn handler() -> ReportSyncEventHandler {
        ReportSyncEventHandler::new(true, Arc::new(Mutex::new(MetricReportManager::new())))
    }
}
