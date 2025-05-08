//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::Result;
use std::path::Path;

#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub fn coredump_configure_kernel(config_path: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use eyre::Context;
        use std::fs::write;

        write(
            "/proc/sys/kernel/core_pattern",
            format!(
                "|/usr/sbin/memfault-core-handler -c {} %P %I %s",
                config_path.display()
            ),
        )
        .wrap_err("Unable to write coredump pattern")
    }
    #[cfg(not(target_os = "linux"))]
    {
        use log::warn;

        warn!("Skipping coredump setting on non-Linux systems.");
        Ok(())
    }
}
