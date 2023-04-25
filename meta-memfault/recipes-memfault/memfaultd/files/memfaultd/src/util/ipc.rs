//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use nix::sys::signal::kill;
use nix::sys::signal::Signal;

use crate::util::pid_file::get_pid_from_file;

pub fn send_flush_queue_signal() -> Result<()> {
    let pid = get_pid_from_file()?;

    kill(pid, Signal::SIGUSR1)?;

    Ok(())
}
