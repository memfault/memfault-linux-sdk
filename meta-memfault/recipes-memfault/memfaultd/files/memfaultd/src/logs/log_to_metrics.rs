//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eyre::Result;
use log::{debug, warn};
use regex::Regex;
use serde_json::{Map, Value};

use crate::{config::LogToMetricRule, metrics::MetricReportManager};

const SEARCH_FIELD: &str = "MESSAGE";

pub struct LogToMetrics {
    rules: Vec<LogToMetricRule>,
    heartbeat_manager: Arc<Mutex<MetricReportManager>>,
    regex_cache: HashMap<String, Regex>,
}

impl LogToMetrics {
    pub fn new(
        rules: Vec<LogToMetricRule>,
        heartbeat_manager: Arc<Mutex<MetricReportManager>>,
    ) -> Self {
        Self {
            rules,
            heartbeat_manager,
            regex_cache: HashMap::new(),
        }
    }

    pub fn process(&mut self, structured_log: &Value) -> Result<()> {
        if let Some(data) = structured_log["data"].as_object() {
            if !self.rules.is_empty() {
                debug!("LogToMetrics: Processing log: {:?}", data);
                for rule in &self.rules {
                    match rule {
                        LogToMetricRule::CountMatching {
                            pattern,
                            metric_name,
                            filter,
                        } => Self::apply_count_matching(
                            data,
                            pattern,
                            &mut self.regex_cache,
                            metric_name,
                            filter,
                            self.heartbeat_manager.clone(),
                        ),
                    }
                }
            }
        }
        Ok(())
    }

    fn get_metric_name_with_captures(metric_name: &str, captures: regex::Captures) -> String {
        let mut metric_name_with_captures = metric_name.to_string();
        for (i, capture) in captures.iter().enumerate() {
            if let Some(capture) = capture {
                metric_name_with_captures =
                    metric_name_with_captures.replace(&format!("${}", i), capture.as_str());
            }
        }
        metric_name_with_captures
    }

    fn apply_count_matching(
        data: &Map<String, Value>,
        pattern: &str,
        regex_cache: &mut HashMap<String, Regex>,
        metric_name: &str,
        filter: &HashMap<String, String>,
        heartbeat_manager: Arc<Mutex<MetricReportManager>>,
    ) {
        // Use filter to quickly disqualify a log entry
        for (key, value) in filter {
            if let Some(log_value) = data.get(key) {
                if log_value != value {
                    return;
                }
            } else {
                return;
            }
        }

        let regex = regex_cache
            .entry(pattern.to_string())
            .or_insert_with(|| Regex::new(pattern).unwrap());
        if let Some(search_value) = data[SEARCH_FIELD].as_str() {
            let captures = regex.captures(search_value);
            debug!(
                "LogToMetrics Pattern '{}'=> MATCH={} Captures={:?}",
                &pattern,
                captures.is_some(),
                captures
            );

            if let Some(captures) = captures {
                let metric_name_with_captures =
                    Self::get_metric_name_with_captures(metric_name, captures);

                if let Err(e) = heartbeat_manager
                    .lock()
                    .unwrap()
                    .increment_counter(&metric_name_with_captures)
                {
                    warn!("Failed to increment metric: {}", e)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{metrics::MetricValue, test_utils::setup_logger};

    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "foo".to_string(),
        metric_name: "foo".to_string(),
        filter: HashMap::default()
    }], vec![json!({"MESSAGE": "foo"})], "foo", 1.0)]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "session opened for user (\\w*)\\(uid=".to_string(),
        metric_name: "ssh_sessions_$1_count".to_string(),
        filter: HashMap::default()
    }], vec![json!({"MESSAGE": "pam_unix(sshd:session): session opened for user thomas(uid=1000) by (uid=0)"})], "ssh_sessions_thomas_count", 1.0)]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "(.*): Scheduled restart job, restart counter is at".to_string(),
        metric_name: "$1_restarts".to_string(),
        filter: HashMap::default()
    }], vec![json!({"MESSAGE": /* systemd[1]: */"docker.service: Scheduled restart job, restart counter is at 1."})], "docker.service_restarts", 1.0)]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "(.*): Scheduled restart job, restart counter is at".to_string(),
        metric_name: "$1_restarts".to_string(),
        filter: HashMap::default()
    }],
    vec![
        json!({"MESSAGE": /* systemd[1]: */"docker.service: Scheduled restart job, restart counter is at 1."}),
        json!({"MESSAGE": /* systemd[1]: */"sshd.service: Scheduled restart job, restart counter is at 1."}),
        json!({"MESSAGE": /* systemd[1]: */"docker.service: Scheduled restart job, restart counter is at 2."}),
    ], "docker.service_restarts", 2.0)
    ]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "(.*): Scheduled restart job, restart counter is at".to_string(),
        metric_name: "$1_restarts".to_string(),
        filter: HashMap::from([("UNIT".to_owned(), "systemd".to_owned())])
    }], vec![json!({"MESSAGE": /* systemd[1]: */"docker.service: Scheduled restart job, restart counter is at 1.", "UNIT": "systemd"})], "docker.service_restarts", 1.0)]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "(.*): Scheduled restart job, restart counter is at".to_string(),
        metric_name: "$1_restarts".to_string(),
        filter: HashMap::from([("_SYSTEMD_UNIT".to_owned(), "ssh.service".to_owned())])
    }], vec![json!({"MESSAGE": /* systemd[1]: */"docker.service: Scheduled restart job, restart counter is at 1.", "_SYSTEMD_UNIT": ""})], "docker.service_restarts", 0.0)]
    #[case(vec![LogToMetricRule::CountMatching {
        pattern: "Out of memory: Killed process \\d+ \\((.*)\\)".to_string(),
        metric_name: "oomkill_$1".to_string(),
        filter: HashMap::default()
    }], vec![json!({"MESSAGE": "Out of memory: Killed process 423 (wefaultd) total-vm:553448kB, anon-rss:284496kB, file-rss:0kB, shmem-rss:0kB, UID:0 pgtables:624kB oom_score_adj:0"})], "oomkill_wefaultd", 1.0)]

    fn test_log_to_metrics(
        #[case] rules: Vec<LogToMetricRule>,
        #[case] logs: Vec<Value>,
        #[case] metric_name: &str,
        #[case] expected_value: f64,
        _setup_logger: (),
    ) {
        let metric_report_manager = Arc::new(Mutex::new(MetricReportManager::new()));
        let mut log_to_metrics = LogToMetrics::new(rules, metric_report_manager.clone());

        for log in logs {
            log_to_metrics
                .process(&json!({ "data": log }))
                .expect("process error");
        }
        let metrics = metric_report_manager
            .lock()
            .unwrap()
            .take_heartbeat_metrics();

        if expected_value == 0.0 {
            assert!(!metrics.iter().any(|m| m.0.as_str() == metric_name));
        } else {
            let m = metrics
                .iter()
                .find(|m| m.0.as_str() == metric_name)
                .unwrap();

            match m.1 {
                MetricValue::Number(v) => assert_eq!(*v, expected_value),
            }
        }
    }
}
