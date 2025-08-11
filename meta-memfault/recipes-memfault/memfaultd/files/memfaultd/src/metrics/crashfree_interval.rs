//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    cmp::max,
    collections::HashMap,
    io::Read,
    iter::once,
    str::{from_utf8, FromStr},
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use eyre::{eyre, Result};
use log::{trace, warn};
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Request, Response};

use crate::{
    cli::NotifyCrashRequest,
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::{
        core_metrics::{
            METRIC_OPERATIONAL_CRASHES, METRIC_OPERATIONAL_CRASHES_PROCESS_PREFIX,
            METRIC_OPERATIONAL_CRASHFREE_HOURS, METRIC_OPERATIONAL_HOURS,
        },
        KeyedMetricReading,
    },
    util::time_measure::TimeMeasure,
};

use super::{MetricStringKey, MetricsMBox};

pub struct CrashFreeIntervalTracker<T: TimeMeasure> {
    last_interval_mark: T,
    last_crashfree_interval_mark: T,
    crash_count: u32,
    process_crash_count: HashMap<String, u64>,
    sender: Sender<CrashInfo<T>>,
    receiver: Receiver<CrashInfo<T>>,
    interval: Duration,
    elapsed_intervals_key: MetricStringKey,
    crashfree_intervals_key: MetricStringKey,
    crash_count_key: MetricStringKey,
    metrics_mbox: MetricsMBox,
}

#[derive(Debug, PartialEq, Eq)]
struct TimeMod<T: TimeMeasure> {
    count: u32,
    mark: T,
}

impl<T> CrashFreeIntervalTracker<T>
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync + 'static,
{
    pub fn new(
        interval: Duration,
        elapsed_intervals_key: &'static str,
        crashfree_intervals_key: &'static str,
        crash_count_key: &'static str,
        metrics_mbox: MetricsMBox,
    ) -> Self {
        let (sender, receiver) = channel();
        Self {
            last_crashfree_interval_mark: T::now(),
            last_interval_mark: T::now(),
            sender,
            receiver,
            crash_count: 0,
            process_crash_count: HashMap::new(),
            interval,
            elapsed_intervals_key: MetricStringKey::from(elapsed_intervals_key),
            crashfree_intervals_key: MetricStringKey::from(crashfree_intervals_key),
            crash_count_key: MetricStringKey::from(crash_count_key),
            metrics_mbox,
        }
    }

    /// Returns a tracker with an hourly interval
    pub fn new_hourly(metrics_mbox: MetricsMBox) -> Self {
        Self::new(
            Duration::from_secs(3600),
            METRIC_OPERATIONAL_HOURS,
            METRIC_OPERATIONAL_CRASHFREE_HOURS,
            METRIC_OPERATIONAL_CRASHES,
            metrics_mbox,
        )
    }

    /// Wait for the next crash or update the metrics if the wait duration has passed.
    ///
    /// This allows us to have instant updates on crashes and hourly updates on the metrics, but
    /// also allows us to periodically update the metrics so that we don't have to wait for a crash.
    pub fn wait_and_update(&mut self, wait_duration: Duration) -> Result<()> {
        if let Ok(crash_info) = self.receiver.recv_timeout(wait_duration) {
            // Drain the receiver to get all crashes that happened since the last update
            self.receiver
                .try_iter()
                .chain(once(crash_info))
                .for_each(|info| {
                    self.crash_count += 1;
                    self.last_crashfree_interval_mark =
                        max(self.last_crashfree_interval_mark, info.timestamp);
                    *self
                        .process_crash_count
                        .entry(info.process_name)
                        .or_insert(0) += 1;
                });
        }

        // Since timing out just means no crashes occurred in the `wait_duration`,
        // update even when the receiver times out.
        self.update()
    }

    fn update(&mut self) -> Result<()> {
        let TimeMod {
            count: count_op_interval,
            mark: last_counted_op_interval,
        } = Self::full_interval_elapsed_since(self.interval, &self.last_interval_mark);
        let TimeMod {
            count: count_crashfree_interval,
            mark: last_counted_crashfree_interval,
        } = Self::full_interval_elapsed_since(self.interval, &self.last_crashfree_interval_mark);

        self.last_interval_mark = last_counted_op_interval;
        self.last_crashfree_interval_mark = last_counted_crashfree_interval;

        let crashes = self.crash_count;
        self.crash_count = 0;

        let mut metrics = vec![
            KeyedMetricReading::add_to_counter(
                self.elapsed_intervals_key.clone(),
                count_op_interval as f64,
            ),
            KeyedMetricReading::add_to_counter(
                self.crashfree_intervals_key.clone(),
                count_crashfree_interval as f64,
            ),
            KeyedMetricReading::add_to_counter(self.crash_count_key.clone(), crashes as f64),
        ];

        let process_crash_metrics = self.process_crash_count.drain().flat_map(
            |(process_name, crash_count)| -> Result<KeyedMetricReading> {
                let operation_crashes_process_key = MetricStringKey::from_str(
                    format!(
                        "{}{}",
                        METRIC_OPERATIONAL_CRASHES_PROCESS_PREFIX, process_name
                    )
                    .as_str(),
                )
                .map_err(|e| eyre!("Couldn't construct MetricStringKey: {}", e))?;
                Ok(KeyedMetricReading::add_to_counter(
                    operation_crashes_process_key,
                    crash_count as f64,
                ))
            },
        );

        metrics.extend(process_crash_metrics);

        trace!("Crashfree hours metrics: {:?}", metrics);

        self.metrics_mbox.send_and_forget(metrics)?;

        Ok(())
    }

    pub fn http_handler(&mut self) -> Box<dyn HttpHandler> {
        Box::new(CrashFreeIntervalHttpHandler {
            channel: self.sender.clone(),
        })
    }

    pub fn channel_handler(&mut self) -> Box<Sender<CrashInfo<T>>> {
        Box::new(self.sender.clone())
    }

    pub fn capture_crash(&self, process_name: String) {
        self.sender
            .send(CrashInfo {
                process_name,
                timestamp: T::now(),
            })
            .expect("Failed to send crash timestamp");
    }

    /// Count how many `interval` have elapsed since `since`.
    ///
    /// This returns the number of intervals that have elapsed since `since`, and the timestamp of the end of the last interval
    /// that was counted. This is the value you should pass as `since` next time you call this function.
    ///
    /// See unit test for examples.
    fn full_interval_elapsed_since(interval: Duration, since: &T) -> TimeMod<T> {
        let now = T::now();
        if *since > now {
            return TimeMod {
                count: 0,
                mark: T::now(),
            };
        }

        let duration = now.since(since);
        let count_interval_elapsed = (duration.as_nanos() / interval.as_nanos()) as u32;
        TimeMod {
            count: count_interval_elapsed,
            mark: since.add(interval * count_interval_elapsed),
        }
    }
}

struct CrashFreeIntervalHttpHandler<T> {
    channel: Sender<CrashInfo<T>>,
}

#[derive(Serialize, Deserialize)]
pub struct CrashInfo<T> {
    pub process_name: String,
    pub timestamp: T,
}

impl<T> CrashFreeIntervalHttpHandler<T>
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync,
{
    fn parse_notify_crash_request(stream: &mut dyn Read) -> Result<NotifyCrashRequest> {
        let mut buf = vec![];
        stream.read_to_end(&mut buf)?;
        let reading = serde_json::from_str(from_utf8(&buf)?)?;
        Ok(reading)
    }
}

impl<T> HttpHandler for CrashFreeIntervalHttpHandler<T>
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync,
{
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() == "/v1/crash/report" && request.method() == &Method::Post {
            match Self::parse_notify_crash_request(request.as_reader()) {
                Ok(NotifyCrashRequest { process_name }) => {
                    self.channel
                        .send(CrashInfo {
                            process_name,
                            timestamp: T::now(),
                        })
                        .expect("Crashfree channel closed");
                }
                Err(e) => {
                    warn!("Failed to parse notify crash request: {}", e);
                    return HttpHandlerResult::Error(format!(
                        "Failed to notify crash request: {}",
                        e
                    ));
                }
            }
            HttpHandlerResult::Response(Response::from_string("OK").boxed())
        } else {
            HttpHandlerResult::NotHandled
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use insta::assert_json_snapshot;
    use rstest::rstest;
    use ssf::ServiceMock;

    use crate::{
        metrics::{
            crashfree_interval::{
                METRIC_OPERATIONAL_CRASHES, METRIC_OPERATIONAL_CRASHFREE_HOURS,
                METRIC_OPERATIONAL_HOURS,
            },
            MetricStringKey, MetricValue, TakeMetrics,
        },
        test_utils::TestInstant,
    };

    use super::CrashFreeIntervalTracker;
    use super::TimeMod;

    #[rstest]
    fn test_counting_intervals() {
        use std::time::Duration;

        // move the clock forward so we can go backwards below
        TestInstant::sleep(Duration::from_secs(3600));
        let now = TestInstant::now();

        let d10 = Duration::from_secs(10);
        assert_eq!(
            CrashFreeIntervalTracker::full_interval_elapsed_since(d10, &now),
            TimeMod {
                count: 0,
                mark: now
            }
        );
        assert_eq!(
            CrashFreeIntervalTracker::full_interval_elapsed_since(
                d10,
                &(now - Duration::from_secs(10))
            ),
            TimeMod {
                count: 1,
                mark: now
            }
        );
        assert_eq!(
            CrashFreeIntervalTracker::full_interval_elapsed_since(
                d10,
                &(now - Duration::from_secs(25))
            ),
            TimeMod {
                count: 2,
                mark: now - Duration::from_secs(5)
            }
        );
    }

    #[rstest]
    fn test_counting_hours() {
        let mut metrics_mock = ServiceMock::new();
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<TestInstant>::new_hourly(metrics_mock.mbox.clone());

        TestInstant::sleep(Duration::from_secs(7200));

        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();

        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 2, 2);
    }

    #[rstest]
    fn test_counting_minutes() {
        let mut metrics_mock = ServiceMock::new();
        let mut crashfree_tracker = CrashFreeIntervalTracker::<TestInstant>::new(
            Duration::from_secs(60),
            METRIC_OPERATIONAL_HOURS,
            METRIC_OPERATIONAL_CRASHFREE_HOURS,
            METRIC_OPERATIONAL_CRASHES,
            metrics_mock.mbox.clone(),
        );

        TestInstant::sleep(Duration::from_secs(3600));
        crashfree_tracker.capture_crash("memfaultd".to_string());
        TestInstant::sleep(Duration::from_secs(3600));

        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 120, 60);
    }

    #[rstest]
    fn test_30min_heartbeat() {
        let mut metrics_mock = ServiceMock::new();
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<TestInstant>::new_hourly(metrics_mock.mbox.clone());

        TestInstant::sleep(Duration::from_secs(1800));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 0, 0);

        TestInstant::sleep(Duration::from_secs(1800));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 1, 1);
    }

    #[rstest]
    fn test_30min_heartbeat_with_crash() {
        let mut metrics_mock = ServiceMock::new();
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<TestInstant>::new_hourly(metrics_mock.mbox.clone());

        TestInstant::sleep(Duration::from_secs(1800));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 0, 0);

        // Crash at t0 + 30min
        crashfree_tracker.capture_crash("memfaultd".to_string());

        // After 30' we should be ready to mark an operational hour
        TestInstant::sleep(Duration::from_secs(1800));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 1, 0);

        // After another 30' we should be ready to mark another crashfree hour
        TestInstant::sleep(Duration::from_secs(1800));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 0, 1);

        // After another 30' we should be ready to mark another operational hour
        TestInstant::sleep(Duration::from_secs(1800));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 1, 0);
    }

    #[rstest]
    fn test_180min_heartbeat_with_one_crash() {
        let mut metrics_mock = ServiceMock::new();
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<TestInstant>::new_hourly(metrics_mock.mbox.clone());

        // Basic test
        TestInstant::sleep(Duration::from_secs(3600 * 3));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 3, 3);

        // Crash at interval + 170'
        TestInstant::sleep(Duration::from_secs(170 * 60));
        crashfree_tracker.capture_crash("memfaultd".to_string());

        // Another 10' to the heartbeat mark
        // We will count 0 operational hour here. That is a consequence of the heartbeat being larger than the hour
        // To avoid this bug, we need to make sure we call the `update` at least once per hour!
        TestInstant::sleep(Duration::from_secs(10 * 60));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 3, 0);

        // However, doing the crash at interval +10' then waiting for 170' will record 2 crashfree hours
        TestInstant::sleep(Duration::from_secs(10 * 60));
        crashfree_tracker.capture_crash("memfaultd".to_string());
        TestInstant::sleep(Duration::from_secs(170 * 60));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_operational_metrics(metrics_mock.take_metrics().unwrap(), 3, 2);
    }

    #[rstest]
    fn test_process_crash_counter() {
        let mut metrics_mock = ServiceMock::new();
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<TestInstant>::new_hourly(metrics_mock.mbox.clone());

        crashfree_tracker.capture_crash("memfaultd".to_string());
        crashfree_tracker.capture_crash("memfaultd".to_string());
        crashfree_tracker.capture_crash("memfaultd".to_string());
        crashfree_tracker.capture_crash("collectd".to_string());

        TestInstant::sleep(Duration::from_secs(3600));
        crashfree_tracker
            .wait_and_update(Duration::from_secs(0))
            .unwrap();
        assert_json_snapshot!(metrics_mock.take_metrics().unwrap());
    }

    fn assert_operational_metrics(
        metrics: BTreeMap<MetricStringKey, MetricValue>,
        expected_op_hours: u32,
        expected_crashfree_hours: u32,
    ) {
        let op_hours = metrics
            .iter()
            .find(|(name, _)| name.as_str() == METRIC_OPERATIONAL_HOURS)
            .map(|(_, value)| value)
            .unwrap();
        let crash_free_hours = metrics
            .iter()
            .find(|(name, _)| name.as_str() == METRIC_OPERATIONAL_CRASHFREE_HOURS)
            .map(|(_, value)| value)
            .unwrap();

        let op_hours_value = match op_hours {
            MetricValue::Number(value) => value,
            _ => panic!("Unexpected metric type"),
        };

        let crashfree_hours_value = match crash_free_hours {
            MetricValue::Number(value) => value,
            _ => panic!("Unexpected metric type"),
        };

        assert_eq!(
            (*op_hours_value as u32, *crashfree_hours_value as u32),
            (expected_op_hours, expected_crashfree_hours)
        );
    }
}
