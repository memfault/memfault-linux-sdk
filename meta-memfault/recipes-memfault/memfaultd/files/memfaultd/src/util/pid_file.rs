//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::id;

use eyre::{Report, Result, WrapErr};
use nix::unistd::Pid;

const PID_FILE: &str = "/var/run/memfaultd.pid";

pub fn write_memfaultd_pid_file() -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(PID_FILE));

    match file {
        Ok(mut file) => writeln!(file, "{}", id()).wrap_err("Failed to write PID file"),
        Err(e) => {
            let msg = match e.kind() {
                ErrorKind::AlreadyExists => "Daemon already running, aborting.",
                _ => "Failed to open PID file, aborting.",
            };
            Err(Report::new(e).wrap_err(msg))
        }
    }
}

pub fn get_pid_from_file() -> Result<Pid> {
    match std::fs::read_to_string(PID_FILE) {
        Ok(pid_string) => {
            let pid = pid_string
                .trim()
                .parse()
                .wrap_err("Failed to parse PID file contents")?;
            Ok(Pid::from_raw(pid))
        }
        Err(e) => {
            eprintln!("Unable to read memfaultd PID file.");
            Err(e.into())
        }
    }
}
