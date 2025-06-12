//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Implements a simple syslog receiver UDP server than can be
//! used to forward messages from a syslog daemon like
//! syslogd or rsyslog to Memfault.
//!
//! Messages are expected to be either in the syslog RFC 5424 or
//! RFC 3164 format, with a single UDP packet per message.
//!
//! The maximum message size is 2048 bytes and longer messages
//! will be truncated.
//!
//! syslog message format RFCs:
//! RFC 3164: https://www.rfc-editor.org/rfc/rfc3164.html
//! RFC 5424: https://www.rfc-editor.org/rfc/rfc5424.html
//!
//! syslog over UDP RFC:
//! https://www.rfc-editor.org/rfc/rfc5426.html
//!
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};

use chrono::{Datelike, Local, TimeZone, Utc};

use eyre::{eyre, Result};
use log::warn;
use ssf::MsgMailbox;
use syslog_loose::{parse_message_with_year_exact_tz, ProcId, SyslogSeverity};

use crate::logs::{
    log_collector::LogEntrySender,
    log_entry::{LogData, LogEntry},
    messages::LogEntryMsg,
};

use super::levels::{
    LOG_LEVEL_CODE_ALERT, LOG_LEVEL_CODE_CRITICAL, LOG_LEVEL_CODE_DEBUG, LOG_LEVEL_CODE_EMERGENCY,
    LOG_LEVEL_CODE_ERROR, LOG_LEVEL_CODE_INFO, LOG_LEVEL_CODE_NOTICE, LOG_LEVEL_CODE_WARN,
};

// From https://www.rfc-editor.org/rfc/rfc5426#section-3.2:
// "All syslog receivers SHOULD be able to receive datagrams
// with message sizes of up to and including 2048 octets."
const MAX_UDP_PACKET_SIZE: usize = 2048;

#[derive(Clone)]
pub struct SyslogServer {}

// TODO: Refactor to use TaskService
impl SyslogServer {
    pub fn run(bind_address: SocketAddr, sender: MsgMailbox<LogEntryMsg>) -> Result<()> {
        let sender = LogEntrySender::new(sender);
        let socket = UdpSocket::bind(bind_address)?;

        loop {
            // From https://www.rfc-editor.org/rfc/rfc5426#section-3.1:
            // "Each syslog UDP datagram MUST contain only one syslog message"
            let mut buf = [0; MAX_UDP_PACKET_SIZE];
            match socket.recv(&mut buf) {
                Ok(amt) => {
                    let message = String::from_utf8_lossy(&buf[..amt]);
                    if let Ok(log_entry) = Self::parse_syslog_message(&message, Local) {
                        if sender.send_entry(log_entry).is_err() {
                            // An error indicates that the channel has been closed, we should
                            // kill this thread.
                            break;
                        }
                    }
                }
                Err(e) => warn!("Syslog server socket error: {}", e),
            }
        }

        Ok(())
    }

    fn priority_code_from_syslog_severity(syslog_severity: SyslogSeverity) -> String {
        match syslog_severity {
            SyslogSeverity::SEV_CRIT => LOG_LEVEL_CODE_CRITICAL.to_string(),
            SyslogSeverity::SEV_ALERT => LOG_LEVEL_CODE_ALERT.to_string(),
            SyslogSeverity::SEV_EMERG => LOG_LEVEL_CODE_EMERGENCY.to_string(),
            SyslogSeverity::SEV_ERR => LOG_LEVEL_CODE_ERROR.to_string(),
            SyslogSeverity::SEV_WARNING => LOG_LEVEL_CODE_WARN.to_string(),
            SyslogSeverity::SEV_INFO => LOG_LEVEL_CODE_INFO.to_string(),
            SyslogSeverity::SEV_DEBUG => LOG_LEVEL_CODE_DEBUG.to_string(),
            SyslogSeverity::SEV_NOTICE => LOG_LEVEL_CODE_NOTICE.to_string(),
        }
    }

    fn resolve_year((month, _date, _hour, _min, _sec): syslog_loose::IncompleteDate) -> i32 {
        let now = Utc::now();
        // If it is now January, messages from December are assumed to be from the previous year
        if now.month() == 1 && month == 12 {
            now.year() - 1
        } else {
            now.year()
        }
    }

    /// Parses a syslog message in either RFC 3164 or RFC 5424 format
    /// `system_timezone` is used as the timezone if it can be obtained
    /// from the system and there is not a timezone specified in the
    /// syslog timestamp
    fn parse_syslog_message<Tz: TimeZone + Copy>(
        raw_message: &str,
        timezone: Tz,
    ) -> Result<LogEntry> {
        let syslog_entry = parse_message_with_year_exact_tz(
            raw_message,
            Self::resolve_year,
            Some(timezone),
            syslog_loose::Variant::Either,
        )
        .map_err(|e| eyre!(e))?;

        let data = LogData {
            message: syslog_entry.msg.to_string(),
            pid: match syslog_entry.procid {
                Some(ProcId::PID(pid)) => Some(pid.to_string()),
                _ => None,
            },
            systemd_unit: match syslog_entry.appname {
                Some(appname) => Some(appname.to_string()),
                // Fallback to hostname if no appname is supplied
                // This is because busybox syslogd puts the application
                // name in the hostname position.
                None => syslog_entry.hostname.map(|hostname| hostname.to_string()),
            },
            priority: syslog_entry
                .severity
                .map(Self::priority_code_from_syslog_severity),
            original_priority: None,
            extra_fields: HashMap::new(),
        };

        let ts = syslog_entry
            .timestamp
            .ok_or_else(|| eyre!("Log has no timestamp"))?
            .into();

        Ok(LogEntry { ts, data })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use insta::assert_json_snapshot;
    use rstest::rstest;

    #[rstest]
    #[case(
        "<34>1 2023-05-15T14:30:45Z hostname systemd 1234 ID - [meta key=\"value\"] System started",
        "system_started_rfc_5424"
    )]
    #[case(
        "<34>May 15 14:30:45 hostname systemd[1234]: System started",
        "system_started_rfc_3164"
    )]
    #[case(
        "<13>May 15 14:30:46 hostname kernel: Kernel panic detected!!",
        "kernel_panic_rfc_3164"
    )]
    fn test_read_syslog_message(#[case] input: &str, #[case] snapshot_name: &str) {
        let result = SyslogServer::parse_syslog_message(input, Utc).unwrap();

        assert_json_snapshot!(snapshot_name, result);
    }

    #[test]
    fn test_missing_timestamp() {
        let input = "<34>hostname systemd[1234]: System started";

        let result = SyslogServer::parse_syslog_message(input, Utc);
        assert!(result.is_err());
    }
}
