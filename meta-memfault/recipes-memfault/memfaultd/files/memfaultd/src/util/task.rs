//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use std::time::{Duration, Instant};

use log::{trace, warn};

use super::time_measure::TimeMeasure;

/// Run `work` every `repeat_interval` while `condition` returns true.
///
/// On error, wait `error_retry` and multiply `error_retry` by 2 every
/// time the error is repeated but never exceeding repeat_interval.
///
/// This is useful if you need (for example) to make a network request at a
/// fixed interval (1 hour) and want to retry sooner (1 minute) if the connection fails.
/// If the connection keeps on failing, the retry time will be increased (2min, 4min, etc).
///
/// When the process receives a signal we will immediately check the condition
/// and run the work if the condition is still true.
/// (You have to catch the signal somewhere - otherwise the process will be terminated.)
pub fn loop_with_exponential_error_backoff<
    W: FnMut() -> Result<()>,
    T: FnMut() -> LoopContinuation,
>(
    work: W,
    condition: T,
    period: Duration,
    error_retry: Duration,
) {
    loop_with_exponential_error_backoff_internal::<_, _, Instant>(
        work,
        condition,
        period,
        error_retry,
        interruptiple_sleep,
    )
}

// std::thread::sleep automatically continues sleeping on SIGINT but we want to be interrupted so we use shuteye::sleep.
fn interruptiple_sleep(d: Duration) {
    shuteye::sleep(d);
}

/// Specify how to continue execution
#[derive(PartialEq, Eq)]
pub enum LoopContinuation {
    /// Continue running the loop normally
    KeepRunning,
    /// Immediately re-process the loop
    RerunImmediately,
    /// Stop running the loop
    Stop,
}

fn loop_with_exponential_error_backoff_internal<
    W: FnMut() -> Result<()>,
    T: FnMut() -> LoopContinuation,
    Time: TimeMeasure,
>(
    mut work: W,
    mut condition: T,
    period: Duration,
    error_retry: Duration,
    sleep: fn(Duration),
) {
    const BACKOFF_MULTIPLIER: u32 = 2;
    let mut count_errors_since_success = 0;
    while condition() != LoopContinuation::Stop {
        let start_work = Time::now();
        let next_run_in = match work() {
            Ok(_) => {
                count_errors_since_success = 0;
                period
            }
            Err(e) => {
                let next_run = Duration::min(
                    error_retry.saturating_mul(
                        BACKOFF_MULTIPLIER.saturating_pow(count_errors_since_success),
                    ),
                    period,
                );

                count_errors_since_success += 1;
                warn!("Error in Memfaultd main loop: {}", e);
                next_run
            }
        };

        if condition() == LoopContinuation::KeepRunning {
            let sleep_maybe = next_run_in.checked_sub(start_work.elapsed());
            if let Some(howlong) = sleep_maybe {
                trace!("Sleep for {:?}", howlong);
                sleep(howlong);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use eyre::eyre;
    use std::cell::{Cell, RefCell};

    use crate::test_utils::TestInstant;

    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::everything_ok(vec![
        TestInvocation {
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_PERIOD),
            run_time: Duration::from_millis(150),
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_PERIOD * 2),
            ..Default::default()
        }
    ])]
    #[case::errors_are_retried_sooner(vec![
        TestInvocation {
            run_time: Duration::from_millis(10),
            is_error: true,
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY),
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY + TEST_PERIOD),
            ..Default::default()
        }
    ])]
    #[case::long_runs_will_rerun_immediately(vec![
        TestInvocation {
            expect_called_at: TestInstant::from(Duration::from_secs(0)),
            run_time: TEST_PERIOD * 10,
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from( TEST_PERIOD * 10),
            ..Default::default()
        }
    ])]
    #[case::errors_retry_backoff(vec![
        TestInvocation {
            run_time: Duration::from_millis(10),
            is_error: true,
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY),
            is_error: true,
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY + TEST_ERROR_RETRY * 2),
            is_error: true,
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY + TEST_ERROR_RETRY * 2 + TEST_ERROR_RETRY * 4),
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY + TEST_ERROR_RETRY * 2 + TEST_ERROR_RETRY * 4 + TEST_PERIOD),
            is_error: true,
            ..Default::default()
        },
        // This one should have reset to normal error retry
        TestInvocation {
            expect_called_at: TestInstant::from(TEST_ERROR_RETRY + TEST_ERROR_RETRY * 2 + TEST_ERROR_RETRY * 4 + TEST_PERIOD + TEST_ERROR_RETRY),
            is_error: true,
            ..Default::default()
        },
    ])]
    #[case::can_rerun_immediately(vec![
        TestInvocation {
            run_time: Duration::from_millis(10),
            is_error: false,
            ..Default::default()
        },
        TestInvocation {
            expect_called_at: TestInstant::from(Duration::from_millis(10)),
            run_immediately: true,
            ..Default::default()
        },
    ])]
    fn test_loop_with_exponential_backoff(#[case] calls: Vec<TestInvocation>) {
        let step = Cell::new(0);
        let call_times = RefCell::new(vec![]);

        let work = || {
            let invocation = &calls[step.get()];

            call_times.borrow_mut().push(TestInstant::now());
            step.set(step.get() + 1);

            TestInstant::sleep(invocation.run_time);

            match invocation.is_error {
                true => Err(eyre!("invocation failed")),
                false => Ok(()),
            }
        };
        // Run until we have executed all the provided steps.
        let condition = || {
            if step.get() < calls.len() {
                // We do not need a +1 here because the step has already been incremented
                // when condition is called after doing the work.
                if step.get() < calls.len() && calls[step.get()].run_immediately {
                    LoopContinuation::RerunImmediately
                } else {
                    LoopContinuation::KeepRunning
                }
            } else {
                LoopContinuation::Stop
            }
        };

        loop_with_exponential_error_backoff_internal::<_, _, TestInstant>(
            work,
            condition,
            TEST_PERIOD,
            TEST_ERROR_RETRY,
            TestInstant::sleep,
        );

        let expected_call_times = calls
            .into_iter()
            .map(|c| c.expect_called_at)
            .collect::<Vec<TestInstant>>();
        assert_eq!(expected_call_times, *call_times.borrow());
    }

    #[derive(Clone)]
    struct TestInvocation {
        run_time: Duration,
        is_error: bool,
        run_immediately: bool,
        expect_called_at: TestInstant,
    }
    impl Default for TestInvocation {
        fn default() -> Self {
            Self {
                run_time: Duration::from_millis(30),
                is_error: false,
                run_immediately: false,
                expect_called_at: TestInstant::from(Duration::from_millis(0)),
            }
        }
    }

    const TEST_PERIOD: Duration = Duration::from_secs(3600);
    const TEST_ERROR_RETRY: Duration = Duration::from_secs(60);
}
