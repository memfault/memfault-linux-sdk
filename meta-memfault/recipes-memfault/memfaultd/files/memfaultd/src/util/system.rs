//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use lazy_static::lazy_static;
use libc::{clockid_t, timespec, CLOCK_MONOTONIC};

#[cfg(target_os = "linux")]
use libc::{sysconf, _SC_CLK_TCK, _SC_PAGE_SIZE};

#[cfg(target_os = "linux")]
use std::fs::read_to_string;

use uuid::Uuid;

lazy_static! {
    static ref OS_RELEASE: Option<String> = read_osrelease().ok();
    static ref OS_TYPE: Option<String> = read_ostype().ok();
}

#[cfg(target_os = "linux")]
pub fn read_system_boot_id() -> Result<Uuid> {
    use eyre::Context;
    use std::str::FromStr;

    const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
    let boot_id = read_to_string(BOOT_ID_PATH);

    match boot_id {
        Ok(boot_id_str) => Uuid::from_str(boot_id_str.trim()).wrap_err("Invalid boot id"),
        Err(_) => Err(eyre!("Unable to read boot id from system.")),
    }
}

#[cfg(target_os = "linux")]
fn read_osrelease() -> Result<String> {
    let mut os_release = read_to_string("/proc/sys/kernel/osrelease")
        .map_err(|e| eyre!("Unable to read osrelease: {}", e))?;
    os_release.truncate(os_release.trim_end().len());
    Ok(os_release)
}

#[cfg(target_os = "linux")]
fn read_ostype() -> Result<String> {
    let mut os_type = read_to_string("/proc/sys/kernel/ostype")
        .map_err(|e| eyre!("Unable to read ostype: {}", e))?;
    os_type.truncate(os_type.trim_end().len());
    Ok(os_type)
}

/// Returns the value stored in /proc/sys/kernel/osrelease
/// The first time it is called we read and cache
/// the value from procfs. On subsequent calls the cached value
/// is returned. This is safe to do since this is a value set
/// at kernel build-time and cannot be altered at runtime.
pub fn get_osrelease() -> Option<String> {
    OS_RELEASE.clone()
}

/// Returns the value stored in /proc/sys/kernel/ostype
/// The first time it is called we read and cache
/// the value from procfs. On subsequent calls the cached value
/// is returned. This is safe to do since this is a value set
/// at kernel build-time and cannot be altered at runtime.
pub fn get_ostype() -> Option<String> {
    OS_TYPE.clone()
}

#[cfg(target_os = "linux")]
pub fn clock_ticks_per_second() -> u64 {
    unsafe { sysconf(_SC_CLK_TCK) as u64 }
}

#[cfg(target_os = "linux")]
pub fn bytes_per_page() -> u64 {
    unsafe { sysconf(_SC_PAGE_SIZE) as u64 }
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

#[cfg(not(target_os = "linux"))]
pub fn read_osrelease() -> Result<String> {
    Ok("non-Linux-system".to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn read_ostype() -> Result<String> {
    Ok("non-Linux-system".to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn clock_ticks_per_second() -> u64 {
    10_000
}

#[cfg(not(target_os = "linux"))]
pub fn bytes_per_page() -> u64 {
    4096
}
