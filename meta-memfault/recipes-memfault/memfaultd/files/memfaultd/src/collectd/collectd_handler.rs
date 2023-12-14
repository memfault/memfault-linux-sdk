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
    metrics::{HeartbeatManager, KeyedMetricReading},
};

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct CollectdHandler {
    data_collection_enabled: bool,
    metrics_store: Arc<Mutex<HeartbeatManager>>,
}

impl CollectdHandler {
    pub fn new(data_collection_enabled: bool, metrics_store: Arc<Mutex<HeartbeatManager>>) -> Self {
        CollectdHandler {
            data_collection_enabled,
            metrics_store,
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
                        if let Err(e) = metrics_store.add_metric(reading) {
                            warn!("Invalid metric: {e}");
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

    use insta::assert_snapshot;
    use rstest::{fixture, rstest};
    use tiny_http::{Method, TestRequest};

    use crate::{
        http_server::{HttpHandler, HttpHandlerResult},
        metrics::HeartbeatManager,
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

        let metrics = handler.metrics_store.lock().unwrap().take_metrics();
        assert_snapshot!(serde_json::to_string_pretty(&metrics)
            .expect("heartbeat_manager should be serializable"));
    }

    #[rstest]
    fn ignores_data_when_data_collection_is_off() {
        let handler = CollectdHandler::new(false, Arc::new(Mutex::new(HeartbeatManager::new())));
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpu","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        let metrics = handler.metrics_store.lock().unwrap().take_metrics();
        assert_snapshot!(serde_json::to_string_pretty(&metrics)
            .expect("heartbeat_manager should be serializable"));
    }

    #[fixture]
    fn handler() -> CollectdHandler {
        CollectdHandler::new(true, Arc::new(Mutex::new(HeartbeatManager::new())))
    }
}
