#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#define PID_FILE "/var/run/memfaultd.pid"

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

bool memfaultd_check_for_pid_file(void);
int memfaultd_get_pid(void);

#ifdef __cplusplus
}
#endif
