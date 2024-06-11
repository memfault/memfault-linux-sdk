//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::Read,
    sync::{Arc, Mutex},
};

use eyre::{eyre, Result};
use log::{debug, log_enabled, trace, warn};
use tiny_http::{Method, Request, Response};

use crate::{
    collectd::payload::Payload,
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::{KeyedMetricReading, MetricReportManager, BUILTIN_SYSTEM_METRIC_NAMESPACES},
};

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct CollectdHandler {
    data_collection_enabled: bool,
    builtin_system_metric_collection_enabled: bool,
    metrics_store: Arc<Mutex<MetricReportManager>>,
    builtin_namespaces: Vec<String>,
}

impl CollectdHandler {
    pub fn new(
        data_collection_enabled: bool,
        builtin_system_metric_collection_enabled: bool,
        metrics_store: Arc<Mutex<MetricReportManager>>,
    ) -> Self {
        CollectdHandler {
            data_collection_enabled,
            builtin_system_metric_collection_enabled,
            metrics_store,
            builtin_namespaces: BUILTIN_SYSTEM_METRIC_NAMESPACES
                .iter()
                .map(|namespace| namespace.to_string() + "/")
                .collect(),
        }
    }

    /// Convert a collectd JSON push (Payload[]) into a list of MetricReading.
    fn parse_request(stream: &mut dyn Read) -> Result<Vec<KeyedMetricReading>> {
        let payloads: Vec<Payload> = if log_enabled!(log::Level::Debug) {
            let mut buf = vec![];
            stream.read_to_end(&mut buf)?;
            let s = String::from_utf8_lossy(&buf);
            trace!("Received JSON: {}", s);
            match serde_json::from_slice(&buf) {
                Ok(payloads) => payloads,
                Err(e) => {
                    debug!("Error parsing JSON: {}\n{}", e, String::from_utf8(buf)?);
                    return Err(eyre!("Error parsing JSON: {}", e));
                }
            }
        } else {
            serde_json::from_reader(stream)?
        };
        Ok(payloads
            .into_iter()
            .flat_map(Vec::<KeyedMetricReading>::from)
            .collect())
    }
}

impl HttpHandler for CollectdHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() != "/v1/collectd" || *request.method() != Method::Post {
            return HttpHandlerResult::NotHandled;
        }
        if self.data_collection_enabled {
            match Self::parse_request(request.as_reader()) {
                Ok(readings) => {
                    let mut metrics_store = self.metrics_store.lock().unwrap();
                    for reading in readings {
                        // If built-in metric collection IS enabled, we need to drop
                        // collectd metric readings who may have overlapping keys with
                        // memfaultd's built-in readings. To be safe, any reading whose
                        // metric key has the same top-level namespace as a built-in system
                        // metric will be dropped
                        //
                        // For example, since CPU metrics can be captured by memfaultd
                        // this conditional will cause us to drop all collectd
                        // metric readings whose keys start with "cpu/" when
                        // built-in system metric collection is enabled
                        if !self.builtin_system_metric_collection_enabled
                            || !self
                                .builtin_namespaces
                                .iter()
                                .any(|namespace| reading.name.as_str().starts_with(namespace))
                        {
                            if let Err(e) = metrics_store.add_metric(reading) {
                                warn!("Invalid metric: {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Error parsing request: {}", e);
                }
            }
        }
        HttpHandlerResult::Response(Response::empty(200).boxed())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use insta::{assert_json_snapshot, assert_snapshot, with_settings};
    use rstest::{fixture, rstest};
    use tiny_http::{Method, TestRequest};

    use crate::{
        http_server::{HttpHandler, HttpHandlerResult},
        metrics::MetricReportManager,
    };

    use super::CollectdHandler;

    #[rstest]
    fn handle_push(handler: CollectdHandler) {
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpu","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let metrics = handler
            .metrics_store
            .lock()
            .unwrap()
            .take_heartbeat_metrics();
        assert_snapshot!(serde_json::to_string_pretty(&metrics)
            .expect("heartbeat_manager should be serializable"));
    }

    #[rstest]
    fn ignores_data_when_data_collection_is_off() {
        let handler = CollectdHandler::new(
            false,
            false,
            Arc::new(Mutex::new(MetricReportManager::new())),
        );
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpu","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let metrics = handler
            .metrics_store
            .lock()
            .unwrap()
            .take_heartbeat_metrics();
        assert_snapshot!(serde_json::to_string_pretty(&metrics)
            .expect("heartbeat_manager should be serializable"));
    }

    #[rstest]
    fn drops_cpu_metrics_when_builtin_system_metrics_are_enabled() {
        let handler =
            CollectdHandler::new(true, true, Arc::new(Mutex::new(MetricReportManager::new())));
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpu","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        // cpufreq should NOT be dropped as it's a different top-level namespace from "cpu"
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpufreq","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"mockplugin","plugin_instance":"0","type":"mock","type_instance":"test"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let metrics = handler
            .metrics_store
            .lock()
            .unwrap()
            .take_heartbeat_metrics();

        with_settings!({sort_maps => true}, {
            assert_json_snapshot!(metrics);
        });
    }

    #[fixture]
    fn handler() -> CollectdHandler {
        CollectdHandler::new(
            true,
            false,
            Arc::new(Mutex::new(MetricReportManager::new())),
        )
    }
}
