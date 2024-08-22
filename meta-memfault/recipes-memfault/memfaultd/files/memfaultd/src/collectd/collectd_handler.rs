//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::Read;

use eyre::{eyre, Result};
use itertools::Itertools;
use log::{debug, log_enabled, trace, warn};
use tiny_http::{Method, Request, Response};

use crate::{
    collectd::payload::Payload,
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::{KeyedMetricReading, MetricsMBox, BUILTIN_SYSTEM_METRIC_NAMESPACES},
};

/// A server that listens for collectd JSON pushes and stores them in memory.
#[derive(Clone)]
pub struct CollectdHandler {
    data_collection_enabled: bool,
    builtin_system_metric_collection_enabled: bool,
    metrics_mbox: MetricsMBox,
    builtin_namespaces: Vec<String>,
}

impl CollectdHandler {
    pub fn new(
        data_collection_enabled: bool,
        builtin_system_metric_collection_enabled: bool,
        metrics_mbox: MetricsMBox,
    ) -> Self {
        CollectdHandler {
            data_collection_enabled,
            builtin_system_metric_collection_enabled,
            metrics_mbox,
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
                Ok(mut readings) => {
                    if self.builtin_system_metric_collection_enabled {
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
                        readings = readings
                            .into_iter()
                            .filter(|reading| {
                                !self
                                    .builtin_namespaces
                                    .iter()
                                    .any(|namespace| reading.name.as_str().starts_with(namespace))
                            })
                            .collect_vec()
                    }

                    if !readings.is_empty() && self.metrics_mbox.send_and_forget(readings).is_err()
                    {
                        return HttpHandlerResult::Response(Response::empty(500).boxed());
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
    use insta::assert_json_snapshot;
    use rstest::{fixture, rstest};
    use ssf::ServiceMock;
    use tiny_http::{Method, TestRequest};

    use crate::{
        http_server::{HttpHandler, HttpHandlerResult},
        metrics::{KeyedMetricReading, TakeMetrics},
    };

    use super::CollectdHandler;

    #[rstest]
    fn handle_push(mut fixture: Fixture) {
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpu","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            fixture.handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        assert_json_snapshot!(fixture.mock.take_metrics().unwrap());
    }

    #[rstest]
    fn ignores_data_when_data_collection_is_off() {
        let mut mock = ServiceMock::new();
        let handler = CollectdHandler::new(false, false, mock.mbox.clone());
        let r = TestRequest::new().with_method(Method::Post).with_path("/v1/collectd").with_body(
            r#"[{"values":[0],"dstypes":["derive"],"dsnames":["value"],"time":1619712000.000,"interval":10.000,"host":"localhost","plugin":"cpu","plugin_instance":"0","type":"cpu","type_instance":"idle"}]"#,
        );
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        assert_eq!(mock.take_messages().len(), 0)
    }

    #[rstest]
    fn drops_cpu_metrics_when_builtin_system_metrics_are_enabled() {
        let mut mock = ServiceMock::new();
        let handler = CollectdHandler::new(true, true, mock.mbox.clone());
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

        assert_json_snapshot!(mock.take_metrics().unwrap());
    }

    struct Fixture {
        handler: CollectdHandler,
        mock: ServiceMock<Vec<KeyedMetricReading>>,
    }

    #[fixture]
    fn fixture() -> Fixture {
        let mock = ServiceMock::new();
        Fixture {
            handler: CollectdHandler::new(true, false, mock.mbox.clone()),
            mock,
        }
    }
}
