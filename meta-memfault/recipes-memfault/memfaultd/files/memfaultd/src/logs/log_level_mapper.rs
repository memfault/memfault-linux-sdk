//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Maps log levels from log messages based on a set of regex rules.
//!
//! The mapper will search for a log level in the message field of a log entry and set the level
//! field to the corresponding log level if a match is found.

use std::borrow::Cow;

use eyre::{eyre, Error, Result};
use regex::{Error as RegexError, Regex};

use crate::config::LevelMappingConfig;

use super::log_entry::LogEntry;

const DEFAULT_EMERG_RULE: &str =
    r"(?i)(\[EMERG(ENCY)?\]|EMERG(ENCY)?:|<EMERG(ENCY)?>|\{EMERG(ENCY)?\}|EMERG(ENCY)? )";
const DEFAULT_ALERT_RULE: &str = r"(?i)(\[ALERT\]|ALERT:|<ALERT>|\{ALERT\}|ALERT )";
const DEFAULT_CRIT_RULE: &str =
    r"(?i)(\[CRIT(ICAL)?\]|CRIT(ICAL)?:|<CRIT(ICAL)?>|\{CRIT(ICAL)?\}|CRIT(ICAL)? )";
const DEFAULT_ERROR_RULE: &str = r"(?i)(\[ERR(OR)?\]|ERR(OR)?:|<ERR(OR)?>|\{ERR(OR)?\}|ERR(OR)? )";
const DEFAULT_WARN_RULE: &str =
    r"(?i)(\[WARN(ING)?\]|WARN(ING)?:|<WARN(ING)?>|\{WARN(ING)?\}|WARN(ING)? )";
const DEFAULT_NOTICE_RULE: &str = r"(?i)(\[NOTICE\]|NOTICE:|<NOTICE>|\{NOTICE\}|NOTICE )";
const DEFAULT_INFO_RULE: &str = r"(?i)(\[INFO\]|INFO:|<INFO>|\{INFO\}|INFO )";
const DEFAULT_DEBUG_RULE: &str =
    r"(?i)(\[DEBUG\]|DEBUG:|<DEBUG>|\{DEBUG\}|DEBUG |\[TRACE\]|TRACE:|<TRACE>|\{TRACE\}|TRACE )";

const LEVEL_COUNT: usize = 8;

const DEFAULT_REGEX_LIST: [&str; LEVEL_COUNT] = [
    DEFAULT_EMERG_RULE,
    DEFAULT_ALERT_RULE,
    DEFAULT_CRIT_RULE,
    DEFAULT_ERROR_RULE,
    DEFAULT_WARN_RULE,
    DEFAULT_NOTICE_RULE,
    DEFAULT_INFO_RULE,
    DEFAULT_DEBUG_RULE,
];

/// Matches log levels in strings and updating metadata.
///
/// This will be run on every log entry to determine the log level based on the message.
/// The regex matches will be matched in descending levels of severity, and the first match will
/// be used to set the log level. If no match is found we will default to the current log level,
/// or none if it is not set.
pub struct LogLevelMapper {
    rules: [Regex; LEVEL_COUNT],
}

impl LogLevelMapper {
    pub fn map_log(&self, log: &mut LogEntry) -> Result<()> {
        for (i, rule) in self.rules.iter().enumerate() {
            if let Cow::Owned(mat) = rule.replace(&log.data.message, "") {
                let data = &mut log.data;
                data.original_priority = data.priority.take();
                data.priority = Some(i.to_string());
                data.message = mat;
                break;
            }
        }

        Ok(())
    }
}

impl TryFrom<&LevelMappingConfig> for LogLevelMapper {
    type Error = Error;

    fn try_from(config: &LevelMappingConfig) -> Result<Self, Self::Error> {
        let rules = config
            .regex
            .as_ref()
            .map_or_else(build_default_rules, |level_regex| {
                Ok([
                    Regex::new(
                        level_regex
                            .emergency
                            .as_deref()
                            .unwrap_or(DEFAULT_EMERG_RULE),
                    )?,
                    Regex::new(level_regex.alert.as_deref().unwrap_or(DEFAULT_ALERT_RULE))?,
                    Regex::new(level_regex.critical.as_deref().unwrap_or(DEFAULT_CRIT_RULE))?,
                    Regex::new(level_regex.error.as_deref().unwrap_or(DEFAULT_ERROR_RULE))?,
                    Regex::new(level_regex.warning.as_deref().unwrap_or(DEFAULT_WARN_RULE))?,
                    Regex::new(level_regex.notice.as_deref().unwrap_or(DEFAULT_NOTICE_RULE))?,
                    Regex::new(level_regex.info.as_deref().unwrap_or(DEFAULT_INFO_RULE))?,
                    Regex::new(level_regex.debug.as_deref().unwrap_or(DEFAULT_DEBUG_RULE))?,
                ])
            });

        match rules {
            Ok(rules) => Ok(Self { rules }),
            Err(err) => Err(eyre!("Failed to build log level mapping rules: {}", err)),
        }
    }
}

fn build_default_rules() -> Result<[Regex; LEVEL_COUNT]> {
    build_rules(DEFAULT_REGEX_LIST)
}

fn build_rules(rule_list: [&str; LEVEL_COUNT]) -> Result<[Regex; LEVEL_COUNT]> {
    let regex = rule_list
        .iter()
        .map(|r| Regex::new(r))
        .collect::<Result<Vec<_>, RegexError>>()?;

    // Safe to unwrap since the number of regexes is fixed
    Ok(regex.try_into().expect("Invalid number of regexes"))
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use crate::config::LevelMappingRegex;
    use crate::logs::log_entry::LogData;

    use super::*;

    use chrono::Utc;
    use itertools::Itertools;
    use rstest::rstest;

    const EMERG_RULE: &str = r"EMERG";
    const ALERT_RULE: &str = r"ALERT";
    const CRIT_RULE: &str = r"CRIT";
    const ERR_RULE: &str = r"ERR";
    const WARN_RULE: &str = r"WARN";
    const NOTICE_RULE: &str = r"NOTICE";
    const INFO_RULE: &str = r"INFO";
    const DEBUG_RULE: &str = r"DEBUG";
    const TEST_RULES: [&str; LEVEL_COUNT] = [
        EMERG_RULE,
        ALERT_RULE,
        CRIT_RULE,
        ERR_RULE,
        WARN_RULE,
        NOTICE_RULE,
        INFO_RULE,
        DEBUG_RULE,
    ];

    #[rstest]
    #[case(EMERG_RULE, "0")]
    #[case(ALERT_RULE, "1")]
    #[case(CRIT_RULE, "2")]
    #[case(ERR_RULE, "3")]
    #[case(WARN_RULE, "4")]
    #[case(NOTICE_RULE, "5")]
    #[case(INFO_RULE, "6")]
    #[case(DEBUG_RULE, "7")]
    fn test_level_match_happy_path(#[case] message: &str, #[case] expected_level: &str) {
        let rules = build_rules(TEST_RULES).unwrap();
        let mapper = LogLevelMapper { rules };

        let original_level = "8".to_string();
        let data = LogData {
            message: message.to_string(),
            pid: None,
            systemd_unit: None,
            priority: Some(original_level.clone()),
            original_priority: None,
            extra_fields: Default::default(),
        };
        let mut entry = LogEntry {
            ts: Utc::now(),
            data,
        };
        mapper.map_log(&mut entry).unwrap();

        // Assert that the level is set
        assert_eq!(
            entry.data.priority.as_ref().unwrap().as_str(),
            expected_level
        );

        // Assert that the original level is saved
        assert_eq!(
            entry.data.original_priority.as_ref().unwrap().as_str(),
            original_level,
        );
    }

    #[test]
    fn test_no_match() {
        let rules = build_rules(TEST_RULES).unwrap();
        let mapper = LogLevelMapper { rules };

        let data = LogData {
            message: "No match".to_string(),
            pid: None,
            systemd_unit: None,
            priority: Some("8".to_string()),
            original_priority: None,
            extra_fields: Default::default(),
        };
        let mut entry = LogEntry {
            ts: Utc::now(),
            data,
        };
        mapper.map_log(&mut entry).unwrap();

        // Assert that the level is not changed
        assert_eq!(entry.data.priority.as_ref().unwrap().as_str(), "8");
    }

    #[test]
    fn test_level_precedence() {
        // Verify that the highest precedence rule is matched
        let rule_strings = [
            "first", "other", "other", "other", "other", "other", "other", "other",
        ];
        let rules = build_rules(rule_strings).unwrap();
        let mapper = LogLevelMapper { rules };

        let mut entry = LogEntry::new_with_message("other");
        mapper.map_log(&mut entry).unwrap();

        // Assert that the second rule is matched
        assert_eq!(entry.data.priority.as_ref().unwrap().as_str(), "1");
    }

    #[rstest]
    #[case(
        r"\[.*\] \[ERROR\]",
        "[2024-09-09 12:00:00] [ERROR] Something went wrong",
        " Something went wrong"
    )]
    fn test_complex_regex(
        #[case] regex: &str,
        #[case] message: &str,
        #[case] expected_message: &str,
    ) {
        let mut rule_strings = ["None"; LEVEL_COUNT];
        rule_strings[3] = regex;

        let rules = build_rules(rule_strings).unwrap();
        let mapper = LogLevelMapper { rules };

        let mut entry = LogEntry::new_with_message(message);
        mapper.map_log(&mut entry).unwrap();

        // Assert that the level is set
        assert_eq!(entry.data.priority.as_ref().unwrap().as_str(), "3");
        assert_eq!(entry.data.message.as_str(), expected_message);
    }

    #[rstest]
    #[case("INFO test message", " test message")]
    #[case("something else WARN test message", "something else  test message")]
    #[case("NO MATCH FOUND", "NO MATCH FOUND")]
    #[case("who would do this? EMERG", "who would do this? ")]
    #[case("who would EMERG do this? EMERG", "who would  do this? EMERG")]
    fn test_match_extraction(#[case] message: &str, #[case] expected_message: &str) {
        let rules = build_rules(TEST_RULES).unwrap();
        let mapper = LogLevelMapper { rules };

        let mut entry = LogEntry::new_with_message(message);
        mapper.map_log(&mut entry).unwrap();

        // Assert that the level is set to INFO
        assert_eq!(entry.data.message, expected_message);
    }

    #[rstest]
    fn test_default_rules() {
        let level_map = [
            ("EMERG", 0),
            ("EMERGENCY", 0),
            ("ALERT", 1),
            ("CRIT", 2),
            ("ERROR", 3),
            ("ERR", 3),
            ("WARN", 4),
            ("WARNING", 4),
            ("NOTICE", 5),
            ("INFO", 6),
            ("DEBUG", 7),
            ("TRACE", 7),
        ];
        let prefix_patterns = ["LEVEL: ", "[LEVEL] ", "<LEVEL> ", "{LEVEL} ", "LEVEL "];
        let message_map = level_map
            .iter()
            .flat_map(|(level, level_num)| {
                // Make lower and uppercase versions of the level
                [
                    (level.to_lowercase(), *level_num),
                    (level.to_uppercase(), *level_num),
                ]
            })
            .cartesian_product(prefix_patterns.iter())
            .map(|((level, level_num), prefix_pattern)| {
                let mut message = prefix_pattern.replace("LEVEL", &level);
                message.push_str("test message");
                let expected_level = level_num.to_string();
                (message, expected_level)
            })
            .collect::<HashMap<_, _>>();

        let rules = build_default_rules().unwrap();
        let mapper = LogLevelMapper { rules };

        for (message, expected_level) in message_map {
            let mut entry = LogEntry::new_with_message(&message);
            mapper
                .map_log(&mut entry)
                .unwrap_or_else(|_| panic!("Failed to map log string {:?}", message));

            assert_eq!(
                entry.data.priority.as_ref().unwrap().as_str(),
                expected_level
            );
        }
    }

    #[rstest]
    fn test_default_fallthrough(#[values(0, 1, 2, 3, 4, 5, 6, 7)] default_level: usize) {
        let rule_strings = (0..LEVEL_COUNT)
            .map(|idx| {
                if idx == default_level {
                    None
                } else {
                    Some("REGEX".to_string())
                }
            })
            .collect::<Vec<_>>();

        let mapping_regex = LevelMappingRegex {
            emergency: rule_strings[0].clone(),
            alert: rule_strings[1].clone(),
            critical: rule_strings[2].clone(),
            error: rule_strings[3].clone(),
            warning: rule_strings[4].clone(),
            notice: rule_strings[5].clone(),
            info: rule_strings[6].clone(),
            debug: rule_strings[7].clone(),
        };
        let level_config = LevelMappingConfig {
            enable: true,
            regex: Some(mapping_regex),
        };
        let mapper = LogLevelMapper::try_from(&level_config).unwrap();

        assert_eq!(
            mapper.rules[default_level].as_str(),
            DEFAULT_REGEX_LIST[default_level]
        );
    }
}
