//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use chrono::Utc;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use memfaultd::config::{LevelMappingConfig, LevelMappingRegex};
use memfaultd::logs::log_entry::{LogData, LogEntry};
use memfaultd::logs::log_level_mapper::LogLevelMapper;

const BENCH_LEVELS: &[&str] = &[
    "EMERG", "ALERT", "CRIT", "ERROR", "WARN", "NOTICE", "INFO", "DEBUG",
];

fn build_log_line(level: &str, message: &str) -> LogEntry {
    let data = LogData {
        message: message.to_string(),
        pid: None,
        systemd_unit: None,
        priority: Some(level.to_string()),
        original_priority: None,
        extra_fields: Default::default(),
    };

    LogEntry {
        ts: Utc::now(),
        data,
    }
}

fn build_regex_string(level: &str) -> String {
    format!(r"\[.*\] \[{}\]:", level)
}

fn build_log_mapper() -> LogLevelMapper {
    let regex = BENCH_LEVELS
        .iter()
        .map(|level| build_regex_string(level))
        .collect::<Vec<_>>();

    let regex = LevelMappingRegex {
        emergency: Some(regex[0].clone()),
        alert: Some(regex[1].clone()),
        critical: Some(regex[2].clone()),
        error: Some(regex[3].clone()),
        warning: Some(regex[4].clone()),
        notice: Some(regex[5].clone()),
        info: Some(regex[6].clone()),
        debug: Some(regex[7].clone()),
    };
    let level_config = LevelMappingConfig {
        enable: true,
        regex: Some(regex),
    };

    LogLevelMapper::try_from(&level_config).expect("Failed to build log level mapper")
}

fn map_logs(num_log_lines: u64, mapper: &LogLevelMapper) {
    for i in 0..num_log_lines {
        let cur_level_idx = i % BENCH_LEVELS.len() as u64;
        let cur_level = BENCH_LEVELS[cur_level_idx as usize];
        let mut log_line = build_log_line(cur_level, "This is a test log message");
        mapper.map_log(&mut log_line).expect("Error mapping log");
    }
}

fn log_mapper_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Log Level Mapper");
    let num_log_lines = [100, 1000];
    let level_mapper = build_log_mapper();

    for num in num_log_lines {
        group.throughput(Throughput::Elements(num));
        group.bench_with_input(BenchmarkId::new("Log Level Mapper", num), &num, |b, num| {
            // Send metrics to preallocate the metrics hashmap
            b.iter(|| {
                map_logs(*num, &level_mapper);
            })
        });
    }
}

criterion_group!(benches, log_mapper_benchmark);
criterion_main!(benches);
