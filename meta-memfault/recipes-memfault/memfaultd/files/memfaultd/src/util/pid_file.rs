//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::{remove_file, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

use eyre::{eyre, Report, Result, WrapErr};
use nix::unistd::Pid;

const PID_FILE: &str = "/var/run/memfaultd.pid";

pub fn write_pid_file() -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(PID_FILE));

    match file {
        Ok(mut file) => writeln!(file, "{}", Pid::this()).wrap_err("Failed to write PID file"),
        Err(e) => {
            let msg = match e.kind() {
                ErrorKind::AlreadyExists => "Daemon already running, aborting.",
                _ => "Failed to open PID file, aborting.",
            };
            Err(Report::new(e).wrap_err(msg))
        }
    }
}

/// Returns true if (and only if) our PID file exists, is readable and contains our current PID.
pub fn is_pid_file_about_me() -> bool {
    match get_pid_from_file() {
        Ok(pid) => pid == Pid::this(),
        _ => false,
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
        Err(_) => Err(eyre!("Couldn't read memfaultd PID file at {}.", PID_FILE)),
    }
}

pub fn remove_pid_file() -> Result<()> {
    remove_file(PID_FILE).wrap_err("Failed to remove PID file")
}

pub fn send_signal_to_pid(signal: nix::sys::signal::Signal) -> Result<()> {
    let pid = get_pid_from_file()?;
    nix::sys::signal::kill(pid, signal).wrap_err("Failed to send signal to memfaultd")
}
