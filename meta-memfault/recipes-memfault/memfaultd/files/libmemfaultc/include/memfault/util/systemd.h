#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd systemd helper

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

bool memfaultd_restart_systemd_service_if_running(const char *service_name);
bool memfaultd_kill_systemd_service(const char *service_name, int signal);

#ifdef __cplusplus
}
#endif
