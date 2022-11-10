#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Utility function to read the current Linux boot ID.

#include <stdbool.h>
#include <uuid/uuid.h>

#ifdef __cplusplus
extern "C" {
#endif

//! Copies the current Linux boot ID into the given buffer.
//! @param boot_id[out] Buffer in which to copy the boot ID.
//! @return True if the boot ID was read successfully, false if not.
bool memfault_linux_boot_id_read(char boot_id[UUID_STR_LEN]);

#ifdef __cplusplus
}
#endif
