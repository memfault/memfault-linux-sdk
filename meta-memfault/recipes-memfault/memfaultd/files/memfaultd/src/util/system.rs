//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use libc::{clockid_t, timespec, CLOCK_MONOTONIC};
use uuid::Uuid;

#[cfg(target_os = "linux")]
pub fn read_system_boot_id() -> Result<Uuid> {
    use eyre::Context;
    use std::{fs::read_to_string, str::FromStr};

    const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
    let boot_id = read_to_string(BOOT_ID_PATH);

    match boot_id {
        Ok(boot_id_str) => Uuid::from_str(boot_id_str.trim()).wrap_err("Invalid boot id"),
        Err(_) => Err(eyre!("Unable to read boot id from system.")),
    }
}

/// Calls clock_gettime
/// Most interesting to us are:
/// CLOCK_MONOTONIC: "clock that increments monotonically, tracking the time
/// since an arbitrary point, and will continue to increment while the system is
/// asleep."
/// CLOCK_BOOTTIME  A  nonsettable system-wide clock that is identical to
/// CLOCK_MONOTONIC, except that it also includes any time that the system is
/// suspended.  This allows applications to get a suspend-aware monotonic clock
/// without having to deal with the complications of CLOCK_REALTIME, which may
/// have discontinuities if the time is changed using settimeofday(2) or
/// similar.
pub enum Clock {
    Monotonic,
    Boottime,
}
pub fn get_system_clock(clock: Clock) -> Result<std::time::Duration> {
    // Linux only so we define it here.
    const CLOCK_BOOTTIME: clockid_t = 7;

    let mut t = timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe {
        libc::clock_gettime(
            match clock {
                Clock::Monotonic => CLOCK_MONOTONIC,
                Clock::Boottime if cfg!(target_os = "linux") => CLOCK_BOOTTIME,
                // Falls back to monotonic if not linux
                Clock::Boottime => CLOCK_MONOTONIC,
            },
            &mut t,
        )
    } != 0
    {
        Err(eyre!("Error getting system clock."))
    } else {
        Ok(std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
    }
}

/// Provide some mock implementations for non-Linux systems. Designed for development. Not actual use.

#[cfg(not(target_os = "linux"))]
pub fn read_system_boot_id() -> Result<Uuid> {
    use once_cell::sync::Lazy;
    static MOCK_BOOT_ID: Lazy<Uuid> = Lazy::new(Uuid::new_v4);
    Ok(*MOCK_BOOT_ID)
}
