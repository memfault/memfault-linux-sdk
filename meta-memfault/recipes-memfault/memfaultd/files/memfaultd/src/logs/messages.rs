//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use ssf::Message;

use super::log_entry::LogEntry;

pub struct FlushLogsMsg;

impl Message for FlushLogsMsg {
    type Reply = Result<()>;
}

pub struct GetQueuedLogsMsg;

impl Message for GetQueuedLogsMsg {
    type Reply = Result<Vec<String>>;
}

pub struct RotateIfNeededMsg;

impl Message for RotateIfNeededMsg {
    type Reply = Result<bool>;
}

impl Message for LogEntry {
    type Reply = Result<()>;
}

pub struct RecoverLogsMsg;

impl Message for RecoverLogsMsg {
    type Reply = Result<bool>;
}

pub struct LogEntryMsg {
    pub entry: LogEntry,
    pub dropped_msg_count: usize,
}

impl LogEntryMsg {
    pub fn new(entry: LogEntry, dropped_msg_count: usize) -> Self {
        Self {
            entry,
            dropped_msg_count,
        }
    }
}

impl Message for LogEntryMsg {
    type Reply = Result<()>;
}
