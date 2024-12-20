//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{collections::HashMap, iter::repeat};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use memfaultd::config::LogToMetricRule;
use memfaultd::logs::log_entry::{LogData, LogEntry};
use memfaultd::logs::log_to_metrics::LogToMetrics;
use memfaultd::metrics::MetricReportManager;
use ssf::ServiceJig;

fn log_line_from_str(line: &str) -> LogEntry {
    let data = LogData {
        message: line.to_string(),
        pid: None,
        systemd_unit: None,
        priority: None,
        original_priority: None,
        extra_fields: HashMap::new(),
    };

    LogEntry {
        ts: chrono::Utc::now(),
        data,
    }
}

fn send_logs(num_log_lines: u64) {
    let report_manager = MetricReportManager::default();
    let report_service = ServiceJig::prepare(report_manager);

    let rules = vec![LogToMetricRule::CountMatching {
        pattern: "eager evaluation is bad".to_string(),
        metric_name: "metric_name".to_string(),
        filter: HashMap::new(),
    }];

    let log_lines = repeat("eager evaluation is bad")
        .take(num_log_lines as usize)
        .map(log_line_from_str)
        .collect::<Vec<LogEntry>>();

    let mut logs_to_metrics = LogToMetrics::new(rules, report_service.mailbox.into());
    log_lines.iter().for_each(|log_line| {
        logs_to_metrics
            .process(log_line)
            .expect("Failed to process log line");
    });
}

fn logs_to_metrics_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Logs to Metrics");
    let num_log_lines = [100, 1000];

    for num in num_log_lines {
        group.throughput(Throughput::Elements(num));
        group.bench_with_input(BenchmarkId::new("Logs to Metrics", num), &num, |b, num| {
            // Send metrics to preallocate the metrics hashmap
            b.iter(|| {
                send_logs(*num);
            })
        });
    }
}

criterion_group!(benches, logs_to_metrics_benchmark);
criterion_main!(benches);
