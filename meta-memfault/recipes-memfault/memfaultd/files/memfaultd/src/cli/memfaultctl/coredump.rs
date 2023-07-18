//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use argh::FromArgValue;
use eyre::Result;

use crate::config::Config;

/// Strategy to trigger a crash.
///
/// This is used to test the crash reporting functionality.
#[derive(Debug)]
pub enum ErrorStrategy {
    SegFault,
    FPException,
}

impl FromArgValue for ErrorStrategy {
    fn from_arg_value(value: &str) -> Result<Self, String> {
        match value {
            "segfault" => Ok(ErrorStrategy::SegFault),
            "divide-by-zero" => Ok(ErrorStrategy::FPException),
            _ => Err("valid strategies are 'segfault' and 'divide-by-zero'".to_string()),
        }
    }
}

pub fn trigger_coredump(config: &Config, error_type: ErrorStrategy) -> Result<()> {
    trigger_coredump_inner(config, error_type)
}

#[cfg(feature = "coredump")]
fn trigger_coredump_inner(config: &Config, error_type: ErrorStrategy) -> Result<()> {
    use crate::util::ipc::send_flush_signal;

    trigger_crash(error_type)?;

    if config.config_file.enable_dev_mode {
        println!("Signaling memfaultd to upload coredump event...");

        // Give the kernel and memfault-core-handler time to process the coredump
        std::thread::sleep(std::time::Duration::from_secs(3));

        send_flush_signal()?;
    }

    Ok(())
}

#[cfg(not(feature = "coredump"))]
fn trigger_coredump_inner(_config: &Config, _error_type: ErrorStrategy) -> Result<()> {
    println!(
        "You must enable the coredump feature when building memfault SDK to report coredumps."
    );

    Ok(())
}

#[cfg(feature = "coredump")]
fn trigger_crash(error_type: ErrorStrategy) -> Result<()> {
    use memfaultc_sys::coredump::memfault_trigger_fp_exception;
    use nix::unistd::{fork, ForkResult};

    println!("Triggering coredump ...");
    match unsafe { fork() } {
        Ok(ForkResult::Parent { .. }) => Ok(()),
        Ok(ForkResult::Child) => {
            match error_type {
                ErrorStrategy::FPException => {
                    // This still needs to be implemented in C because Rust automatically
                    // generates code to prevent divide-by-zero errors. This can be moved to Rust when
                    // [unchecked_div](https://doc.rust-lang.org/std/intrinsics/fn.unchecked_div.html)
                    // is stabilized.
                    unsafe {
                        memfault_trigger_fp_exception();
                    }
                }
                ErrorStrategy::SegFault => {
                    unsafe { std::ptr::null_mut::<i32>().write(42) };
                }
            }

            unreachable!("Child process should have crashed");
        }
        Err(e) => Err(eyre::eyre!("Failed to fork process: {}", e)),
    }
}
