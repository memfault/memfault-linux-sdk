//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    io::Read,
    str::{from_utf8, FromStr},
};

use eyre::Result;
use ssf::MsgMailbox;
use tiny_http::{Method, Request, Response};

use crate::metrics::battery::BatteryReadingMessage;
use crate::{
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::BatteryMonitorReading,
};

/// A server that listens for battery reading pushes and stores them in memory.
#[derive(Clone)]
pub struct BatteryReadingHandler {
    data_collection_enabled: bool,
    battery_monitor_mbox: MsgMailbox<BatteryReadingMessage>,
}

impl BatteryReadingHandler {
    pub fn new(
        data_collection_enabled: bool,
        battery_monitor_mbox: MsgMailbox<BatteryReadingMessage>,
    ) -> Self {
        Self {
            data_collection_enabled,
            battery_monitor_mbox,
        }
    }

    fn parse_request(stream: &mut dyn Read) -> Result<BatteryMonitorReading> {
        let mut buf = vec![];
        stream.read_to_end(&mut buf)?;
        let reading = BatteryMonitorReading::from_str(from_utf8(&buf)?)?;
        Ok(reading)
    }
}

impl HttpHandler for BatteryReadingHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() != "/v1/battery/add_reading" || *request.method() != Method::Post {
            return HttpHandlerResult::NotHandled;
        }
        if self.data_collection_enabled {
            match Self::parse_request(request.as_reader()) {
                Ok(reading) => {
                    match self
                        .battery_monitor_mbox
                        .send_and_forget(BatteryReadingMessage::new(reading))
                    {
                        Ok(()) => HttpHandlerResult::Response(Response::empty(200).boxed()),
                        Err(e) => HttpHandlerResult::Error(format!(
                            "Failed to add battery reading to metrics: {:#}",
                            e
                        )),
                    }
                }
                Err(e) => HttpHandlerResult::Error(format!(
                    "Failed to parse battery reading string: {}",
                    e
                )),
            }
        } else {
            HttpHandlerResult::Response(Response::empty(200).boxed())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use insta::assert_json_snapshot;
    use rstest::rstest;
    use ssf::{ServiceJig, ServiceMock};
    use tiny_http::{Method, TestRequest};

    use crate::{
        http_server::{HttpHandler, HttpHandlerResult},
        metrics::BatteryMonitor,
    };
    use crate::{metrics::TakeMetrics, test_utils::TestInstant};

    #[rstest]
    fn handle_push() {
        let mut metrics_mock = ServiceMock::new();
        let mut battery_monitor = ServiceJig::prepare(BatteryMonitor::<TestInstant>::new(
            metrics_mock.mbox.clone(),
        ));

        let handler = BatteryReadingHandler::new(true, battery_monitor.mailbox.clone().into());
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/battery/add_reading")
            .with_body("Charging:80");
        assert!(matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Response(_)
        ));

        battery_monitor.process_all();

        assert_json_snapshot!(metrics_mock.take_metrics().unwrap());
    }

    // Need to include a test_name string parameter here due to
    // a known issue using insta and rstest together:
    // https://github.com/la10736/rstest/issues/183
    #[rstest]
    #[case(vec!["Charging:80", "Charging:90", "Full:100", "Discharging:95", "Discharging:85"], 30, "charging_then_discharging")]
    #[case(vec!["Full:100", "Discharging:90", "Discharging:50", "Not charging:50", "Discharging:30", "Discharging:10", "Charging:50"], 30, "nonconsecutive_discharges")]
    #[case(vec!["Charging:90", "Charging:92.465", "Unknown:91.78", "Discharging:90", "Discharging:80"], 30, "non_integer_percentages")]
    fn handle_push_of_multiple_readings(
        #[case] readings: Vec<&'static str>,
        #[case] seconds_between_readings: u64,
        #[case] test_name: &str,
    ) {
        let mut metrics_mock = ServiceMock::new();
        let mut battery_monitor = ServiceJig::prepare(BatteryMonitor::<TestInstant>::new(
            metrics_mock.mbox.clone(),
        ));
        let handler = BatteryReadingHandler::new(true, battery_monitor.mailbox.clone().into());
        for reading in readings {
            let r = TestRequest::new()
                .with_method(Method::Post)
                .with_path("/v1/battery/add_reading")
                .with_body(reading);
            assert!(matches!(
                handler.handle_request(&mut r.into()),
                HttpHandlerResult::Response(_)
            ));

            battery_monitor.process_all();
            TestInstant::sleep(Duration::from_secs(seconds_between_readings));
        }

        // Set battery_soc_pct to 0.0 to avoid flakiness due to it being weighted by wall time
        assert_json_snapshot!(test_name, metrics_mock.take_metrics().unwrap(), {".battery_soc_pct" => 0.0 });
    }

    #[rstest]
    fn errors_when_body_is_invalid() {
        let mock = ServiceMock::new();
        let handler = BatteryReadingHandler::new(true, mock.mbox);
        let r = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/v1/battery/add_reading")
            .with_body("{\"state\": \"Charging\", \"percent\":80}");
        matches!(
            handler.handle_request(&mut r.into()),
            HttpHandlerResult::Error(_)
        );
    }
}
