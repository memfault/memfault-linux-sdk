#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include "config.h"

#ifdef __cplusplus
extern "C" {
#endif

int memfault_set_runtime_bool_and_reload(sMemfaultdConfig *config, const char *config_key,
                                         const char *description, bool value);

#ifdef __cplusplus
}
#endif
