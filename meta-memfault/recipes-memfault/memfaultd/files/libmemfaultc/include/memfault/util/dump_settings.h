#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Print the current configuration, runtime settings and build time settings to the console.

#ifdef __cplusplus
extern "C" {
#endif

void memfaultd_dump_settings(sMemfaultdDeviceSettings *settings, sMemfaultdConfig *config,
                             const char *config_file);

#ifdef __cplusplus
}
#endif
