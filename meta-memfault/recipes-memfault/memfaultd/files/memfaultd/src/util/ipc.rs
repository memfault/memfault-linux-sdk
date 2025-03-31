//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use nix::sys::signal::Signal::{SIGUSR1, SIGUSR2};

use super::pid_file::send_signal_to_pid;

pub fn send_flush_signal(skip_serialization: bool) -> Result<()> {
    send_signal_to_pid(if skip_serialization { SIGUSR2 } else { SIGUSR1 })
}
