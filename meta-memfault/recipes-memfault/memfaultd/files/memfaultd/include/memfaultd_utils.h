//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd helper util function definitions

#pragma once

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

bool memfaultd_utils_restart_service_if_running(const char *src_module, const char *service_name);

#ifdef __cplusplus
}
#endif
