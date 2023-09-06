//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use nix::sys::signal::Signal::SIGHUP;

use crate::util::pid_file::send_signal_to_pid;

use super::MemfaultdServiceManager;

pub struct DefaultServiceManager;

impl MemfaultdServiceManager for DefaultServiceManager {
    fn restart_memfaultd_if_running(&self) -> eyre::Result<()> {
        send_signal_to_pid(SIGHUP)
    }

    fn service_manager_status(&self) -> eyre::Result<super::ServiceManagerStatus> {
        Ok(super::ServiceManagerStatus::Unknown)
    }
}
