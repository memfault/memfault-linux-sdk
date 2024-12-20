//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Defines the format of the stacktrace output.
//!
//! All structs here represent the final JSON output that is written to the MAR entry.
//!
//! NOTE: All numerical values are represented as strings to avoid any truncation or loss of
//! precision when converting to JSON. This is due to the fact that JSON assumes all values
//! are signed.

use serde::{Deserialize, Serialize};

const STACKTRACE_FORMAT_VERSION: &str = "1";

#[derive(Debug, Serialize, Deserialize)]
/// Output format for the stacktrace.
pub struct StacktraceFormat {
    version: String,
    signal: String,
    cmdline: String,
    symbols: Vec<SymbolFileDescriptor>,
    threads: Vec<ThreadDescriptor>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    core_handler_logs: Vec<String>,
}

impl StacktraceFormat {
    pub fn new(
        signal: String,
        cmdline: String,
        symbols: Vec<SymbolFileDescriptor>,
        threads: Vec<ThreadDescriptor>,
        core_handler_logs: Vec<String>,
    ) -> Self {
        Self {
            version: STACKTRACE_FORMAT_VERSION.to_string(),
            signal,
            cmdline,
            symbols,
            threads,
            core_handler_logs,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
/// Describes a symbol file needed to symbolicate a stacktrace.
pub struct SymbolFileDescriptor {
    pc_range: PcRange,
    build_id: String,
    compiled_offset: String,
    runtime_offset: String,
    path: String,
}

impl SymbolFileDescriptor {
    pub fn new(
        start_addr: usize,
        end_addr: usize,
        build_id: String,
        compiled_offset: usize,
        runtime_offset: usize,
        path: String,
    ) -> Self {
        let pc_range = PcRange {
            start: format!("{:#x}", start_addr),
            end: format!("{:#x}", end_addr),
        };

        Self {
            pc_range,
            build_id,
            compiled_offset: format!("{:#x}", compiled_offset),
            runtime_offset: format!("{:#x}", runtime_offset),
            path,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
/// A range of program counters indicating which symbol file to load
pub struct PcRange {
    start: String,
    end: String,
}

#[derive(Debug, Serialize, Deserialize)]
/// Describes a thread in the stacktrace.
pub struct ThreadDescriptor {
    active: bool,
    pcs: Vec<String>,
}

impl ThreadDescriptor {
    pub fn new(active: bool, pcs: Vec<usize>) -> Self {
        let pcs = pcs.into_iter().map(|pc| format!("{:#x}", pc)).collect();
        Self { active, pcs }
    }
}
