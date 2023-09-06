//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use nix::sys::signal::Signal::SIGUSR1;

use super::pid_file::send_signal_to_pid;

pub fn send_flush_signal() -> Result<()> {
    send_signal_to_pid(SIGUSR1)
}
