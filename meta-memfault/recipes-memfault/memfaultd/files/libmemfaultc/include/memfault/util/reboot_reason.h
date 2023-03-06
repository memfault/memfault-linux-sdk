#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include <stdbool.h>

#include "memfault/core/reboot_reason_types.h"

#ifdef __cplusplus
extern "C" {
#endif

bool memfaultd_is_reboot_reason_valid(eMemfaultRebootReason reboot_reason);
const char *memfaultd_reboot_reason_str(eMemfaultRebootReason reboot_reason);

#ifdef __cplusplus
}
#endif
