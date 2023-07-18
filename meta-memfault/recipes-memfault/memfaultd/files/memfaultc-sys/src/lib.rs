//
// Copyright (c) Memfault, Inc.
// See License.txt for details
// main functions

#[cfg(feature = "coredump")]
pub mod coredump;

#[cfg(feature = "systemd")]
pub mod systemd;

#[cfg(feature = "swupdate")]
pub mod swupdate;
