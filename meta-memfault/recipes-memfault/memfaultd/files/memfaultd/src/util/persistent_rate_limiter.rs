//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::{DateTime, Duration, TimeZone, Utc};
use eyre::Result;
use itertools::Itertools;
use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

/// A `PersistentRateLimiter` that enforces action limits based on time
/// duration.
///
/// - It allows actions up to the specified `count` within the `duration`.
/// - It stores a history of attempts in a file `path`.
pub struct PersistentRateLimiter {
    path: PathBuf,
    count: u32,
    duration: Duration,
    /// A list of all the times the rate limiter has been hit. We keep them
    /// sorted from most recent to oldest and we cap this list to count.
    history: VecDeque<DateTime<Utc>>,
}

impl PersistentRateLimiter {
    /// Load the rate limiter state from disk.
    ///
    /// Non-existent file is not considered an error. Garbage in the file will be skipped over.
    pub fn load<P: AsRef<Path>>(path: P, count: u32, duration: Duration) -> Result<Self> {
        if count == 0 {
            return Err(eyre::eyre!("count must be greater than 0"));
        }
        if duration.num_milliseconds() == 0 {
            return Err(eyre::eyre!("duration must be greater than 0"));
        }

        // Load previous invocations of the rate limiter, discarding anything that is not parseable.
        let history = match File::open(&path) {
            Ok(file) => BufReader::new(file)
                .split(b' ')
                .filter_map::<DateTime<Utc>, _>(|t| {
                    let ts: i64 = std::str::from_utf8(&t.ok()?).ok()?.parse().ok()?;
                    Utc.timestamp_opt(ts, 0).single()
                })
                .sorted_by(|a, b| b.cmp(a))
                .collect::<VecDeque<_>>(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: path.as_ref().to_owned(),
            count,
            duration,
            history,
        })
    }

    fn check_with_time(&mut self, now: DateTime<Utc>) -> bool {
        if self.history.len() >= self.count as usize {
            if let Some(oldest) = self.history.back() {
                if now.signed_duration_since(*oldest) < self.duration {
                    return false;
                }
            }
        }

        self.history.push_front(now);
        self.history.truncate(self.count as usize);

        true
    }

    /// Check if the rate limiter will allow one call now. The state is updated
    /// but not written to disk. Call `save()` to persist the rate limiter.
    pub fn check(&mut self) -> bool {
        self.check_with_time(Utc::now())
    }

    /// Writes the rate limiter state to disk.
    pub fn save(self) -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.path)?;
        for time in self.history.iter() {
            write!(file, "{} ", time.timestamp())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tempfile::tempdir;

    use super::*;

    #[rstest]
    #[case::invalid_count(0, Duration::seconds(1))]
    #[case::invalid_duration(1, Duration::seconds(0))]
    fn invalid_init(#[case] count: u32, #[case] duration: Duration) {
        let tmpdir = tempdir().unwrap();
        let path = tmpdir.path().join("test");

        assert!(PersistentRateLimiter::load(path, count, duration).is_err());
    }

    #[rstest]
    #[case(1, Duration::seconds(10), vec![0, 10, 20, 30, 35, 40], vec![true, true, true, true, false, true])]
    #[case(3, Duration::seconds(10), vec![0, 0, 9, 10, 11, 12], vec![true, true, true, true, true, false ])]
    #[case(3, Duration::seconds(10), vec![0, 0, 9, 9, 9, 9, 18, 19], vec![true, true, true, false, false, false, true, true ])]
    #[case(3, Duration::seconds(10), vec![0, 100, 200, 300, 400, 500, 600], vec![true, true, true, true, true, true, true])]
    fn test_rate_limiter(
        #[case] count: u32,
        #[case] duration: Duration,
        #[case] timestamps: Vec<i64>,
        #[case] expected: Vec<bool>,
    ) {
        assert_eq!(
            timestamps.len(),
            expected.len(),
            "timestamps and expected results should be the same length"
        );

        let tmpdir = tempdir().unwrap();
        let path = tmpdir.path().join("test");

        for (time, expected) in timestamps.into_iter().zip(expected.into_iter()) {
            let mut limiter =
                PersistentRateLimiter::load(&path, count, duration).expect("load error");
            assert_eq!(
                limiter.check_with_time(Utc.timestamp_opt(time, 0).single().unwrap()),
                expected,
                "time: {} - history: {:?}",
                time,
                limiter.history
            );
            limiter.save().expect("save error");
        }
    }
}
