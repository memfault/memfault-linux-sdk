#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Utility to persist the boot id of the last tracked reboot to a file.

#include <stdbool.h>
#include <uuid/uuid.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Returns whether the given boot id has not yet been tracked.
 * @param last_tracked_boot_id_file The path to the file in which the last tracked boot id is
 * stored.
 * @param current_boot_id The UUID string of the current boot id.
 * @return True if the reboot for the given boot id has not been tracked yet.
 */
bool memfault_reboot_is_untracked_boot_id(const char *last_tracked_boot_id_file,
                                          const char *current_boot_id);

#ifdef __cplusplus
}
#endif
