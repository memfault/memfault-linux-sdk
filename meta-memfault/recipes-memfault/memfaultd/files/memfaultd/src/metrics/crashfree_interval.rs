//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    cmp::max,
    iter::once,
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

use chrono::Utc;
use tiny_http::{Method, Request, Response};

use crate::{
    http_server::{HttpHandler, HttpHandlerResult},
    metrics::{
        core_metrics::{
            METRIC_OPERATIONAL_CRASHES, METRIC_OPERATIONAL_CRASHFREE_HOURS,
            METRIC_OPERATIONAL_HOURS,
        },
        MetricReading,
    },
    util::time_measure::TimeMeasure,
};

use super::{KeyedMetricReading, MetricStringKey};

pub struct CrashFreeIntervalTracker<T: TimeMeasure> {
    last_interval_mark: T,
    last_crashfree_interval_mark: T,
    crash_count: u32,
    sender: Sender<T>,
    receiver: Receiver<T>,
    interval: Duration,
    elapsed_intervals_key: MetricStringKey,
    crashfree_intervals_key: MetricStringKey,
    crash_count_key: MetricStringKey,
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
        elapsed_intervals_key: MetricStringKey,
        crashfree_intervals_key: MetricStringKey,
        crash_count_key: MetricStringKey,
    ) -> Self {
        let (sender, receiver) = channel();
        Self {
            last_crashfree_interval_mark: T::now(),
            last_interval_mark: T::now(),
            sender,
            receiver,
            crash_count: 0,
            interval,
            elapsed_intervals_key,
            crashfree_intervals_key,
            crash_count_key,
        }
    }

    /// Returns a tracker with an hourly interval
    pub fn new_hourly() -> Self {
        Self::new(
            Duration::from_secs(3600),
            METRIC_OPERATIONAL_HOURS.parse().unwrap(),
            METRIC_OPERATIONAL_CRASHFREE_HOURS.parse().unwrap(),
            METRIC_OPERATIONAL_CRASHES.parse().unwrap(),
        )
    }

    /// Wait for the next crash or update the metrics if the wait duration has passed.
    ///
    /// This allows us to have instant updates on crashes and hourly updates on the metrics, but
    /// also allows us to periodically update the metrics so that we don't have to wait for a crash.
    pub fn wait_and_update(&mut self, wait_duration: Duration) -> Vec<KeyedMetricReading> {
        if let Ok(crash_ts) = self.receiver.recv_timeout(wait_duration) {
            // Drain the receiver to get all crashes that happened since the last update
            self.receiver
                .try_iter()
                .chain(once(crash_ts))
                .for_each(|ts| {
                    self.crash_count += 1;
                    self.last_crashfree_interval_mark = max(self.last_crashfree_interval_mark, ts);
                });
        }

        // Since timing out just means no crashes occurred in the `wait_duration`,
        // update even when the receiver times out.
        self.update()
    }

    fn update(&mut self) -> Vec<KeyedMetricReading> {
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

        let metrics_ts = Utc::now();
        vec![
            KeyedMetricReading::new(
                self.elapsed_intervals_key.clone(),
                MetricReading::Counter {
                    value: count_op_interval as f64,
                    timestamp: metrics_ts,
                },
            ),
            KeyedMetricReading::new(
                self.crashfree_intervals_key.clone(),
                MetricReading::Counter {
                    value: count_crashfree_interval as f64,
                    timestamp: metrics_ts,
                },
            ),
            KeyedMetricReading::new(
                self.crash_count_key.clone(),
                MetricReading::Counter {
                    value: crashes as f64,
                    timestamp: metrics_ts,
                },
            ),
        ]
    }

    pub fn http_handler(&mut self) -> Box<dyn HttpHandler> {
        Box::new(CrashFreeIntervalHttpHandler {
            channel: self.sender.clone(),
        })
    }

    pub fn capture_crash(&self) {
        self.sender
            .send(T::now())
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
    channel: Sender<T>,
}

impl<T> HttpHandler for CrashFreeIntervalHttpHandler<T>
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync,
{
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() == "/v1/crash/report" && request.method() == &Method::Post {
            self.channel.send(T::now()).unwrap();
            HttpHandlerResult::Response(Response::from_string("OK").boxed())
        } else {
            HttpHandlerResult::NotHandled
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rstest::rstest;

    use crate::{
        metrics::{
            crashfree_interval::{
                METRIC_OPERATIONAL_CRASHES, METRIC_OPERATIONAL_CRASHFREE_HOURS,
                METRIC_OPERATIONAL_HOURS,
            },
            KeyedMetricReading, MetricReading,
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
        let mut crashfree_tracker = CrashFreeIntervalTracker::<TestInstant>::new_hourly();

        TestInstant::sleep(Duration::from_secs(7200));

        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            2,
            2,
        );
    }

    #[rstest]
    fn test_counting_minutes() {
        let mut crashfree_tracker = CrashFreeIntervalTracker::<TestInstant>::new(
            Duration::from_secs(60),
            METRIC_OPERATIONAL_HOURS.parse().unwrap(),
            METRIC_OPERATIONAL_CRASHFREE_HOURS.parse().unwrap(),
            METRIC_OPERATIONAL_CRASHES.parse().unwrap(),
        );

        TestInstant::sleep(Duration::from_secs(3600));
        crashfree_tracker.capture_crash();
        TestInstant::sleep(Duration::from_secs(3600));

        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            120,
            60,
        );
    }

    #[rstest]
    fn test_30min_heartbeat() {
        let mut crashfree_tracker = CrashFreeIntervalTracker::<TestInstant>::new_hourly();

        TestInstant::sleep(Duration::from_secs(1800));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            0,
            0,
        );

        TestInstant::sleep(Duration::from_secs(1800));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            1,
            1,
        );
    }

    #[rstest]
    fn test_30min_heartbeat_with_crash() {
        let mut crashfree_tracker = CrashFreeIntervalTracker::<TestInstant>::new_hourly();

        TestInstant::sleep(Duration::from_secs(1800));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            0,
            0,
        );

        // Crash at t0 + 30min
        crashfree_tracker.capture_crash();

        // After 30' we should be ready to mark an operational hour
        TestInstant::sleep(Duration::from_secs(1800));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            1,
            0,
        );

        // After another 30' we should be ready to mark another crashfree hour
        TestInstant::sleep(Duration::from_secs(1800));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            0,
            1,
        );

        // After another 30' we should be ready to mark another operational hour
        TestInstant::sleep(Duration::from_secs(1800));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            1,
            0,
        );
    }

    #[rstest]
    fn test_180min_heartbeat_with_one_crash() {
        let mut crashfree_tracker = CrashFreeIntervalTracker::<TestInstant>::new_hourly();

        // Basic test
        TestInstant::sleep(Duration::from_secs(3600 * 3));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            3,
            3,
        );

        // Crash at interval + 170'
        TestInstant::sleep(Duration::from_secs(170 * 60));
        crashfree_tracker.capture_crash();

        // Another 10' to the heartbeat mark
        // We will count 0 operational hour here. That is a consequence of the heartbeat being larger than the hour
        // To avoid this bug, we need to make sure we call the `update` at least once per hour!
        TestInstant::sleep(Duration::from_secs(10 * 60));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            3,
            0,
        );

        // However, doing the crash at interval +10' then waiting for 170' will record 2 crashfree hours
        TestInstant::sleep(Duration::from_secs(10 * 60));
        crashfree_tracker.capture_crash();
        TestInstant::sleep(Duration::from_secs(170 * 60));
        assert_operational_metrics(
            crashfree_tracker.wait_and_update(Duration::from_secs(0)),
            3,
            2,
        );
    }

    fn assert_operational_metrics(
        metrics: Vec<KeyedMetricReading>,
        expected_op_hours: u32,
        expected_crashfree_hours: u32,
    ) {
        assert_eq!(metrics.len(), 3);
        let op_hours = metrics
            .iter()
            .find(|m| m.name.as_str() == METRIC_OPERATIONAL_HOURS)
            .unwrap();
        let crash_free_hours = metrics
            .iter()
            .find(|m| m.name.as_str() == METRIC_OPERATIONAL_CRASHFREE_HOURS)
            .unwrap();

        let op_hours_value = match op_hours.value {
            MetricReading::Counter { value, .. } => value,
            _ => panic!("Unexpected metric type"),
        };

        let crashfree_hours_value = match crash_free_hours.value {
            MetricReading::Counter { value, .. } => value,
            _ => panic!("Unexpected metric type"),
        };

        assert_eq!(
            (op_hours_value as u32, crashfree_hours_value as u32),
            (expected_op_hours, expected_crashfree_hours)
        );
    }
}
