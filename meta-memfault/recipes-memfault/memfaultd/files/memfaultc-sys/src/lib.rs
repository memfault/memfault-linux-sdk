//
// Copyright (c) Memfault, Inc.
// See License.txt for details
// main functions

#[cfg(feature = "coredump")]
pub mod coredump;

#[cfg(all(feature = "systemd", not(target_os = "macos")))]
pub mod systemd;
#[cfg(all(feature = "systemd", target_os = "macos"))]
pub mod systemd_mock;

#[cfg(all(feature = "systemd", target_os = "macos"))]
pub use systemd_mock as systemd;

#[cfg(feature = "swupdate")]
pub mod swupdate;
