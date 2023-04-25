//
// Copyright (c) Memfault, Inc.
// See License.txt for details
pub mod cli;
mod config;
#[cfg(feature = "logging")]
mod fluent_bit;
#[cfg(feature = "logging")]
mod logs;
pub mod mar;
mod memfaultd;
pub mod metrics;
mod network;
#[cfg(feature = "coredump")]
mod process_coredumps;
mod queue;
mod retriable_error;
mod service;
#[cfg(test)]
mod test_utils;
pub mod util;

pub mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}
