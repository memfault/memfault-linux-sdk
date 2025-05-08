#![deny(clippy::print_stdout, clippy::print_stderr)]
//
// Copyright (c) Memfault, Inc.
// See License.txt for details

pub mod cli;
mod collectd;
pub mod config;
#[cfg(feature = "coredump")]
mod coredump;
#[cfg(feature = "logging")]
mod fluent_bit;

pub mod http_server;
#[cfg(feature = "logging")]
pub mod logs;
pub mod mar;
mod memfaultd;
pub mod metrics;
mod mmc;
mod network;
mod reboot;
mod retriable_error;
mod service_manager;
#[cfg(feature = "swupdate")]
mod swupdate;
#[cfg(test)]
mod test_utils;
pub mod util;

pub mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}
