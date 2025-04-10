//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::{hash_map::Entry, HashMap},
    str::FromStr,
};

use eyre::{eyre, Result};
use log::warn;

use regex::{Captures, Regex};

use crate::{
    config::{LogFilterRule, LogRuleAction, LogToMetricRule},
    metrics::{KeyedMetricReading, MetricStringKey, MetricsMBox},
};

use super::log_entry::LogEntry;

#[derive(Clone, Debug)]
pub enum LogRuleResult {
    Excluded,
    Included,
    Passed,
    NonMatch,
}

pub const SYSTEMD_UNIT_KEY: &str = "_SYSTEMD_UNIT";
pub const PRIORITY_KEY: &str = "PRIORITY";

impl From<&LogRuleAction> for LogRuleResult {
    fn from(value: &LogRuleAction) -> Self {
        match value {
            LogRuleAction::Pass => LogRuleResult::Passed,
            LogRuleAction::Include => LogRuleResult::Included,
            LogRuleAction::Exclude => LogRuleResult::Excluded,
        }
    }
}

fn level_str_from_priority(priority: &str) -> &'static str {
    match priority {
        "0" => "EMERGENCY",
        "1" => "ALERT",
        "2" => "CRITICAL",
        "3" => "ERROR",
        "4" => "WARN",
        "5" => "NOTICE",
        "6" => "INFO",
        "7" => "DEBUG",
        _ => {
            warn!("LogToMetrics rule has invalid priority: {}", priority);
            "UNKNOWN"
        }
    }
}

impl From<LogToMetricRule> for LogFilterRule {
    fn from(log_to_metric_rule: LogToMetricRule) -> Self {
        match log_to_metric_rule {
            LogToMetricRule::CountMatching {
                pattern,
                metric_name,
                filter,
            } => {
                let mut log_to_metric_filters = filter;
                let service = log_to_metric_filters.remove(SYSTEMD_UNIT_KEY);
                let priority = log_to_metric_filters.remove(PRIORITY_KEY);
                let level = priority.map(|p| level_str_from_priority(p.as_str()));

                Self {
                    counter_name: Some(metric_name),
                    service,
                    pattern: Some(pattern),
                    level: level.map(|l| l.to_string()),
                    extra_fields: Some(log_to_metric_filters),
                    action: Some(LogRuleAction::Pass),
                }
            }
        }
    }
}

pub struct LogFilter {
    rules: Vec<LogFilterRule>,
    default_action: LogRuleAction,
    metrics_mbox: MetricsMBox,
    regex_cache: HashMap<String, Regex>,
}

enum MessageMatchResult {
    NonMatch,
    Match,
    MatchWithCounterKey(MetricStringKey),
}

impl LogFilter {
    pub fn new(
        log_filter_rules: Vec<LogFilterRule>,
        log_to_metrics_rules: Vec<LogToMetricRule>,
        default_action: LogRuleAction,
        metrics_mbox: MetricsMBox,
    ) -> Self {
        // log_to_metrics rules and the default counter rules all have the
        // "pass" action, so they should be inserted first (so they will
        // increment their counters even if a later LogFilter rule filters
        // the matching log entry out)
        let mut rules: Vec<LogFilterRule> = log_to_metrics_rules
            .into_iter()
            .map(LogFilterRule::from)
            .collect::<Vec<_>>();
        rules.extend(Self::default_log_filter_counters());

        // Since these rules can filter out log entries, they
        // should come at the end of the vector
        rules.extend(log_filter_rules);

        Self {
            rules,
            default_action,
            metrics_mbox,
            regex_cache: HashMap::new(),
        }
    }

    pub fn new_no_default_rules(
        log_filter_rules: Vec<LogFilterRule>,
        log_to_metrics_rules: Vec<LogToMetricRule>,
        default_action: LogRuleAction,
        metrics_mbox: MetricsMBox,
    ) -> Self {
        let mut rules: Vec<LogFilterRule> = log_to_metrics_rules
            .into_iter()
            .map(LogFilterRule::from)
            .collect::<Vec<_>>();
        rules.extend(log_filter_rules);

        Self {
            rules,
            default_action,
            metrics_mbox,
            regex_cache: HashMap::new(),
        }
    }

    /// Returns a default vector of LogFilterRules that are
    /// generally applicable across Linux systems.
    /// These rules all have the Pass action and simply
    /// increment a counter when a matching log message
    /// is processed.
    fn default_log_filter_counters() -> Vec<LogFilterRule> {
        vec![
            // Count systemd restarts per-service
            LogFilterRule {
                counter_name: Some("systemd_restarts_$1".to_string()),
                pattern: Some("(.*): Scheduled restart job, restart counter is at".to_string()),
                service: Some("init.scope".to_string()),
                extra_fields: None,
                level: None,
                action: Some(LogRuleAction::Pass),
            },
            // Count OOM kills per-process
            LogFilterRule {
                counter_name: Some("oomkill_$1".to_string()),
                pattern: Some("Out of memory: Killed process \\d+ \\((.*)\\)".to_string()),
                service: None,
                extra_fields: None,
                level: None,
                action: Some(LogRuleAction::Pass),
            },
        ]
    }

    /// Applies all the rules in the LogFilter to the specified LogEntry
    /// If the log entry should be filtered out based on the rules,
    /// None is returned. Otherwise the log entry wrapped in a Some is returned.
    ///
    /// This method also increments any logs to metrics counters whose patterns
    /// match the specified LogEntry
    pub fn apply_rules(&mut self, log_entry: LogEntry) -> Option<LogEntry> {
        for rule in &self.rules {
            match Self::apply_rule(
                rule,
                &log_entry,
                &self.default_action,
                &self.metrics_mbox,
                &mut self.regex_cache,
            ) {
                // Exit when a match against a rule with a
                // Include or Exclude action is hit
                LogRuleResult::Excluded => return None,
                LogRuleResult::Included => return Some(log_entry),
                // Otherwise continue onto next rule
                LogRuleResult::Passed => continue,
                LogRuleResult::NonMatch => continue,
            }
        }

        // No rules matched - fall back to default action
        if self.default_action == LogRuleAction::Exclude {
            None
        } else {
            Some(log_entry)
        }
    }

    fn apply_rule(
        rule: &LogFilterRule,
        log_entry: &LogEntry,
        default_action: &LogRuleAction,
        metrics_mbox: &MetricsMBox,
        regex_cache: &mut HashMap<String, Regex>,
    ) -> LogRuleResult {
        // Check log level and service first to limit how many regex matches we run
        if Self::level_matches(rule, log_entry)
            && Self::service_matches(rule, log_entry)
            && Self::extra_fields_match(rule, log_entry)
        {
            match Self::try_match_message_pattern(rule, log_entry, regex_cache) {
                Ok(MessageMatchResult::MatchWithCounterKey(counter_key)) => {
                    if let Err(e) = metrics_mbox
                        .send_and_forget(vec![KeyedMetricReading::increment_counter(counter_key)])
                    {
                        warn!("Failed to increment logs to metrics counter {}", e);
                    };
                    rule.action
                        .as_ref()
                        .map_or_else(|| LogRuleResult::from(default_action), LogRuleResult::from)
                }
                Ok(MessageMatchResult::Match) => rule
                    .action
                    .as_ref()
                    .map_or_else(|| LogRuleResult::from(default_action), LogRuleResult::from),
                Ok(MessageMatchResult::NonMatch) => LogRuleResult::NonMatch,
                Err(e) => {
                    warn!(
                        "Failed to match log message {:?} with pattern {:?}: {}",
                        log_entry, rule, e
                    );
                    LogRuleResult::NonMatch
                }
            }
        } else {
            LogRuleResult::NonMatch
        }
    }

    /// Matches the systemd_unit field of the LogEntry's data struct against
    /// the LogFilterRule's service field. All services match for a
    /// LogFilterRule that does not have a service defined.
    fn service_matches(rule: &LogFilterRule, entry: &LogEntry) -> bool {
        match &rule.service {
            Some(rule_service) => entry
                .data
                .systemd_unit
                .as_ref()
                .is_some_and(|entry_service| *entry_service == *rule_service),
            None => true,
        }
    }

    /// Matches the level field of the LogEntry's data struct against
    /// the LogFilterRule's priority field. All levels match for a
    /// LogFilterRule that does not have a level defined.
    fn level_matches(rule: &LogFilterRule, entry: &LogEntry) -> bool {
        match &rule.level {
            Some(rule_level) => entry.data.priority.as_ref().is_some_and(|entry_priority| {
                Self::priority_matches_level_str(entry_priority.as_str(), rule_level.as_str())
            }),
            None => true,
        }
    }

    /// Given a journald priority code and a log level string,
    /// returns two if they are considered within the same level of
    /// severity.
    ///
    /// Docs on systemd journal priority codes: https://wiki.archlinux.org/title/Systemd/Journal
    fn priority_matches_level_str(priority: &str, level_str: &str) -> bool {
        match priority {
            "0" => level_str == "EMERGENCY",
            "1" => level_str == "ALERT",
            "2" => level_str == "CRITICAL",
            "3" => level_str == "ERROR",
            "4" => level_str == "WARN",
            "5" => level_str == "NOTICE",
            "6" => level_str == "INFO",
            "7" => level_str == "DEBUG",
            _ => false,
        }
    }

    /// Matches the systemd_unit field of the LogEntry's data struct against
    /// the LogFilterRule's service field. All services match for a
    /// LogFilterRule that does not have a service defined.
    fn extra_fields_match(rule: &LogFilterRule, entry: &LogEntry) -> bool {
        match &rule.extra_fields {
            Some(fields) => fields.iter().all(|(key, value)| {
                entry
                    .data
                    .get_field(key.as_str())
                    .is_some_and(|log_value| log_value == *value)
            }),
            None => true,
        }
    }

    /// Matches the message field of the LogEntry's data struct against
    /// the LogFilterRule's pattern field. All messages match for a
    /// LogFilterRule that does not have a pattern defined.
    ///
    /// While this function is responsible for constructing
    /// the MetricStringKey that should be incremented (if configured),
    /// the actual incrementing is the responsibility of the caller.
    /// The construction of the MetricStringKey is done inside this
    /// function to keep ownership of the Regex::Captures object simple.
    fn try_match_message_pattern(
        rule: &LogFilterRule,
        entry: &LogEntry,
        regex_cache: &mut HashMap<String, Regex>,
    ) -> Result<MessageMatchResult> {
        match &rule.pattern {
            Some(pattern) => {
                // Cache Regex so we only need to call Regex::new once per pattern
                let r = match regex_cache.entry(pattern.clone()) {
                    Entry::Occupied(e) => e.into_mut(),
                    Entry::Vacant(e) => e.insert(Regex::new(pattern)?),
                };
                let capture_result = r.captures(entry.data.message.as_str());
                match capture_result {
                    Some(captures) => {
                        // There is a "counter_name" in the rule, so populate the
                        // key pattern with the captures from the regex match
                        if let Some(counter_key_pattern) = &rule.counter_name {
                            let counter_key = MetricStringKey::from_str(
                                Self::get_metric_name_with_captures(
                                    counter_key_pattern.as_str(),
                                    captures,
                                )
                                .as_str(),
                            )
                            .map_err(|e| eyre!("Couldn't construct MetricStringKey: {}", e))?;
                            Ok(MessageMatchResult::MatchWithCounterKey(counter_key))
                        } else {
                            Ok(MessageMatchResult::Match)
                        }
                    }
                    None => Ok(MessageMatchResult::NonMatch),
                }
            }
            // If "pattern" isn't set all messages are a match
            None => {
                if let Some(counter_name) = &rule.counter_name {
                    Ok(MessageMatchResult::MatchWithCounterKey(
                        MetricStringKey::from_str(counter_name.as_str())
                            .map_err(|e| eyre!("Couldn't construct MetricStringKey: {}", e))?,
                    ))
                } else {
                    Ok(MessageMatchResult::Match)
                }
            }
        }
    }

    fn get_metric_name_with_captures(metric_name: &str, captures: Captures) -> String {
        let mut metric_name_with_captures = metric_name.to_string();
        for (i, capture) in captures.iter().enumerate() {
            if let Some(capture) = capture {
                metric_name_with_captures =
                    metric_name_with_captures.replace(&format!("${}", i), capture.as_str());
            }
        }
        metric_name_with_captures
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_json_snapshot;
    use rstest::rstest;

    use super::*;
    use crate::{logs::log_entry::LogValue, metrics::TakeMetrics};
    use ssf::ServiceMock;

    #[rstest]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "test_service", "7"),
        LogRuleAction::Include
    )]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "test_service", "7"),
        LogRuleAction::Exclude
    )]
    fn no_rules_default_action(#[case] log_entry: LogEntry, #[case] default_action: LogRuleAction) {
        let service = ServiceMock::new();
        let sender = service.mbox;
        let mut log_filter = LogFilter::new(vec![], vec![], default_action.clone(), sender);
        if default_action == LogRuleAction::Include {
            assert!(log_filter.apply_rules(log_entry).is_some());
        } else {
            assert!(log_filter.apply_rules(log_entry).is_none());
        }
    }

    #[rstest]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "test_service", "7"),
        LogRuleAction::Include
    )]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "test_service", "7"),
        LogRuleAction::Exclude
    )]
    fn rule_match_message_pattern_overrides_default(
        #[case] log_entry: LogEntry,
        #[case] default_action: LogRuleAction,
    ) {
        let service = ServiceMock::new();
        let sender = service.mbox;
        let log_rule = LogFilterRule {
            service: None,
            counter_name: None,
            pattern: Some("test .*".to_string()),
            level: None,
            extra_fields: None,
            action: Some(LogRuleAction::Include),
        };
        let mut log_filter = LogFilter::new(vec![log_rule], vec![], default_action, sender);
        assert!(log_filter.apply_rules(log_entry).is_some());
    }

    #[rstest]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "7", "test_service"),
        LogRuleAction::Include
    )]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "7", "test_service"),
        LogRuleAction::Exclude
    )]
    fn rule_match_service_overrides_default(
        #[case] log_entry: LogEntry,
        #[case] default_action: LogRuleAction,
    ) {
        let service = ServiceMock::new();
        let sender = service.mbox;
        let log_rule = LogFilterRule {
            service: Some("test_service".to_string()),
            counter_name: None,
            pattern: None,
            level: None,
            extra_fields: None,
            action: Some(LogRuleAction::Include),
        };
        let mut log_filter = LogFilter::new(vec![log_rule], vec![], default_action, sender);
        assert!(log_filter.apply_rules(log_entry).is_some());
    }

    #[rstest]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "7", "test_service"),
        LogRuleAction::Include
    )]
    #[case(
        LogEntry::new_with_message_level_and_service("test message", "7", "test_service"),
        LogRuleAction::Exclude
    )]
    fn rule_match_priority_overrides_default(
        #[case] log_entry: LogEntry,
        #[case] default_action: LogRuleAction,
    ) {
        let service = ServiceMock::new();
        let sender = service.mbox;
        let log_rule = LogFilterRule {
            service: None,
            counter_name: None,
            pattern: None,
            level: Some("DEBUG".to_string()),
            extra_fields: None,
            action: Some(LogRuleAction::Include),
        };
        let mut log_filter = LogFilter::new(vec![log_rule], vec![], default_action, sender);
        assert!(log_filter.apply_rules(log_entry).is_some());
    }

    #[rstest]
    #[case(
        vec![LogEntry::new_with_message_level_and_service("test message", "7", "test_service")],
        vec![serde_json::from_str::<LogFilterRule>("{\"level\": \"DEBUG\", \"action\": \"exclude\"}").expect("Couldn't deserialize test case")],
        "one_line_one_rule_matching"
    )]
    #[case(
        vec![LogEntry::new_with_message_level_and_service("test message", "7", "test_service")],
        vec![
            serde_json::from_str::<LogFilterRule>("{\"level\": \"DEBUG\", \"action\": \"exclude\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"pattern\": \"test .*\", \"action\": \"include\"}").expect("Couldn't deserialize test case")
        ],
        "earlier_rules_take_priority"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("another message", "6", "test_service"),
            LogEntry::new_with_message_level_and_service("one more message", "6", "test_service"),
            LogEntry::new_with_message_level_and_service("test message", "7", "test_service"),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>("{\"pattern\": \"another .*\", \"action\": \"include\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"pattern\": \".* message\", \"action\": \"exclude\"}").expect("Couldn't deserialize test case"),
        ],
        "multiple_log_lines"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("another message", "6", "test_service"),
            LogEntry::new_with_message_level_and_service("one more message", "6", "test_service"),
            LogEntry::new_with_message_level_and_service("test message", "7", "test_service"),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>("{\"level\": \"DEBUG\", \"action\": \"exclude\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"pattern\": \"another .*\", \"action\": \"include\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"pattern\": \".* message\", \"action\": \"exclude\"}").expect("Couldn't deserialize test case"),
        ],
        "multiple_log_lines_mixed_rules"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("Knock, knock", "4", "room_service"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "6", "table_service"),
            LogEntry::new_with_message_level_and_service("What can I help you with today?", "4", "customer_service"),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>("{\"level\": \"WARN\", \"counter_name\": \"warning_log_lines\", \"action\": \"pass\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"pattern\": \".* I .*\", \"action\": \"pass\", \"counter_name\": \"first_person_log_lines\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"level\": \"INFO\", \"action\": \"exclude\"}").expect("Couldn't deserialize test case"),
        ],
        "simple_logs_to_metrics"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user blake(uid=1001)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user pat(uid=1002)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>(r#"{"pattern": "session opened for user (\\w*)\\(uid=", "action": "pass", "counter_name": "ssh_sessions_$1_count"}"#).expect("Couldn't deserialize test case")
        ],
        "logs_to_metrics_with_dynamic_counters"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("Out of memory: Killed process 29116 (wefaultd) total-vm:518632kB, anon-rss:255728kB, file-rss:0kB, shmem-rss:0kB, UID:0 pgtables:568kB oom_score_adj:0", "6", ""),
            LogEntry::new_with_message_level_and_service("Out of memory: Killed process 29116 (wefaultd) total-vm:518632kB, anon-rss:255728kB, file-rss:0kB, shmem-rss:0kB, UID:0 pgtables:568kB oom_score_adj:0", "6", ""),
            LogEntry::new_with_message_level_and_service("wefaultd.service: Scheduled restart job, restart counter is at 263.", "6", "init.scope"),
            LogEntry::new_with_message_level_and_service("memfaultd.service: Scheduled restart job, restart counter is at 110.", "6", "init.scope"),
            LogEntry::new_with_message_level_and_service("Out of memory: Killed process 29116 (collectd) total-vm:518632kB, anon-rss:255728kB, file-rss:0kB, shmem-rss:0kB, UID:0 pgtables:568kB oom_score_adj:0", "6", ""),
            LogEntry::new_with_message_level_and_service("Out of memory: Killed process 29116 (wefaultd) total-vm:518632kB, anon-rss:255728kB, file-rss:0kB, shmem-rss:0kB, UID:0 pgtables:568kB oom_score_adj:0", "6", ""),
            LogEntry::new_with_message_level_and_service("wefaultd.service: Scheduled restart job, restart counter is at 264.", "6", "init.scope"),
        ],
        vec![],
        "test_default_rules"
    )]
    fn test_filter_rules(
        #[case] log_entries: Vec<LogEntry>,
        #[case] log_rules: Vec<LogFilterRule>,
        #[case] test_name: &str,
    ) {
        let mut service = ServiceMock::new();
        let sender = service.mbox.clone();
        let mut log_filter = LogFilter::new(log_rules, vec![], LogRuleAction::Include, sender);
        assert_json_snapshot!(
            test_name,
            log_entries
                .into_iter()
                .flat_map(|entry| log_filter.apply_rules(entry))
                .collect::<Vec<_>>()
        );

        assert_json_snapshot!(
            format!("{}-metrics", test_name).as_str(),
            service.take_metrics().unwrap()
        );
    }

    #[rstest]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user blake(uid=1001)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user pat(uid=1002)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>(r#"{"pattern": "session opened for user (\\w*)\\(uid=", "action": "pass", "counter_name": "ssh_sessions_$1_count"}"#).expect("Couldn't deserialize test case")
        ],
        vec![
            LogToMetricRule::CountMatching { pattern: "May I .*".to_string(), metric_name: "polite_questions".to_string(), filter: HashMap::new()}
        ],
        "legacy_logs_to_metrics_config"
    )]
    #[rstest]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user blake(uid=1001)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user pat(uid=1002)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("May I please take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>(r#"{"pattern": "May I .*", "action": "exclude"}"#).expect("Couldn't deserialize test case")
        ],
        vec![
            LogToMetricRule::CountMatching { pattern: "May I .*".to_string(), metric_name: "polite_questions".to_string(), filter: HashMap::new()}
        ],
        "legacy_logs_to_metrics_applied_before_filter_rules"
    )]
    #[rstest]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user blake(uid=1001)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user pat(uid=1002)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("May I please take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "6", "sshd"),
        ],
        vec![
        ],
        vec![
            LogToMetricRule::CountMatching { pattern: "session opened for user (\\w*)\\(uid=".to_string(), metric_name: "ssh_sessions_$1_count".to_string(), filter: HashMap::new()}
        ],
        "legacy_logs_to_metrics_matches_filter_rules"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "0", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user blake(uid=1001)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "0", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "0", "sshd"),
            LogEntry::new_with_message_level_and_service("session opened for user pat(uid=1002)", "6", "sshd"),
            LogEntry::new_with_message_level_and_service("May I take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("May I please take your order?", "7", "table_service"),
            LogEntry::new_with_message_level_and_service("session opened for user thomas(uid=1000)", "0", "sshd"),
        ],
        vec![
        ],
        vec![
            LogToMetricRule::CountMatching { pattern: "session opened for user (\\w*)\\(uid=".to_string(), metric_name: "ssh_sessions_$1_count".to_string(), filter: HashMap::from_iter([(SYSTEMD_UNIT_KEY.to_string(), "sshd".to_string()), (PRIORITY_KEY.to_string(), "0".to_string())])}
        ],
        "legacy_logs_to_metrics_with_service_and_priority"
    )]
    fn test_filter_rules_with_log_to_metrics(
        #[case] log_entries: Vec<LogEntry>,
        #[case] log_rules: Vec<LogFilterRule>,
        #[case] legacy_log_to_metric_rules: Vec<LogToMetricRule>,
        #[case] test_name: &str,
    ) {
        let mut service = ServiceMock::new();
        let sender = service.mbox.clone();
        let mut log_filter = LogFilter::new(
            log_rules,
            legacy_log_to_metric_rules,
            LogRuleAction::Include,
            sender,
        );
        assert_json_snapshot!(
            test_name,
            log_entries
                .into_iter()
                .flat_map(|entry| log_filter.apply_rules(entry))
                .collect::<Vec<_>>()
        );

        assert_json_snapshot!(
            format!("{}-metrics", test_name).as_str(),
            service.take_metrics().unwrap()
        );
    }

    #[rstest]
    #[case(
        vec![LogEntry::new_with_message_and_extra_fields("Hello, world!", HashMap::from_iter([("foo".to_string(), LogValue::String("bar".to_string()))]))],
        vec![serde_json::from_str::<LogFilterRule>("{\"extra_fields\": {\"foo\": \"bar\"}, \"action\": \"exclude\", \"counter_name\": \"foobar_count\"}").expect("Couldn't deserialize test case")],
        "simple_with_extra_fields"
    )]
    #[case(
        vec![
            LogEntry::new_with_message_and_extra_fields("Hello, world!", HashMap::from_iter([("foo".to_string(), LogValue::String("bar".to_string()))])),
            LogEntry::new_with_message_and_extra_fields("Goodbye, world!", HashMap::from_iter([("foo".to_string(), LogValue::String("baz".to_string()))])),
            LogEntry::new_with_message_and_extra_fields("Hello again, world!", HashMap::from_iter([("foo".to_string(), LogValue::String("baz".to_string()))])),
            LogEntry::new_with_message_and_extra_fields("Hello one more time, world!", HashMap::from_iter([("foo".to_string(), LogValue::String("bar".to_string()))])),
            LogEntry::new_with_message_and_extra_fields("Goodbye for real, world!", HashMap::from_iter([("foo".to_string(), LogValue::String("qux".to_string()))])),
        ],
        vec![
            serde_json::from_str::<LogFilterRule>("{\"extra_fields\": {\"foo\": \"bar\"}, \"action\": \"pass\", \"pattern\": \"Hello.*\", \"counter_name\": \"foobar_hello_count\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"extra_fields\": {\"foo\": \"baz\"}, \"action\": \"pass\", \"pattern\": \"Goodbye.*\", \"counter_name\": \"foobaz_goodbye_count\"}").expect("Couldn't deserialize test case"),
            serde_json::from_str::<LogFilterRule>("{\"action\": \"exclude\", \"pattern\": \"Goodbye .*\", \"counter_name\": \"total_goodbye_count\"}").expect("Couldn't deserialize test case"),
        ],
        "multiple_rules_with_extra_fields"
    )]
    fn test_filter_rules_with_extra_fields(
        #[case] log_entries: Vec<LogEntry>,
        #[case] log_rules: Vec<LogFilterRule>,
        #[case] test_name: &str,
    ) {
        let mut service = ServiceMock::new();
        let sender = service.mbox.clone();
        let mut log_filter = LogFilter::new(log_rules, vec![], LogRuleAction::Include, sender);
        assert_json_snapshot!(
            test_name,
            log_entries
                .into_iter()
                .flat_map(|entry| log_filter.apply_rules(entry))
                .collect::<Vec<_>>()
        );

        assert_json_snapshot!(
            format!("{}-metrics", test_name).as_str(),
            service.take_metrics().unwrap()
        );
    }
}
