#![allow(clippy::print_stdout, clippy::print_stderr)]
//
// Copyright (c) Memfault, Inc.
// See License.txt for details

use eyre::{eyre, Result};
use log::LevelFilter;
use std::env::args;
use std::path::Path;
use stderrlog::{LogLevelNum, StdErrLog};

#[cfg(all(target_os = "linux", feature = "coredump"))]
mod memfault_core_handler;
#[cfg(feature = "mfw")]
mod memfault_watch;
mod memfaultctl;
mod memfaultd;
mod memfaultd_client;
mod show_settings;
mod version;

pub use memfaultd_client::*;

fn build_logger(level: LevelFilter) -> StdErrLog {
    let mut log = stderrlog::new();

    log.module("memfaultd");
    log.verbosity(LogLevelNum::from(level));

    log
}

fn init_logger(level: LevelFilter) -> Result<()> {
    build_logger(level)
        .init()
        .map_err(|e| eyre!("Failed to initialize logger: {}", e))
}

pub fn main() {
    let cmd_name = args().next().and_then(|arg0| {
        Path::new(&arg0)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
    });

    let result = match cmd_name.as_deref() {
        #[cfg(all(target_os = "linux", feature = "coredump"))]
        Some("memfault-core-handler") => memfault_core_handler::main(),
        #[cfg(not(all(target_os = "linux", feature = "coredump")))]
        Some("memfault-core-handler") => Err(eyre!(
            "memfault-core-handler is not supported in this build"
        )),
        Some("memfaultctl") => memfaultctl::main(),
        Some("memfaultd") => memfaultd::main(),
        #[cfg(feature = "mfw")]
        Some("mfw") => memfault_watch::main(),
        #[cfg(not(feature = "mfw"))]
        Some("mfw") => Err(eyre!("Memfault-watch is currently experimental. You must compile with the experimental flag enabled.")),
        Some(cmd_name) => Err(eyre!(
            "Unknown command: {}. Should be memfaultd/memfaultctl/memfault-core-handler.",
            cmd_name
        )),
        None => Err(eyre!("No command name found")),
    };

    match result {
        Ok(_) => (),
        Err(e) => {
            eprintln!("{:#}", e);
            std::process::exit(-1);
        }
    }
}
