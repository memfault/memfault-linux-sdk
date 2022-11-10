#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Utility to process pstore files after booting.

#ifdef __cplusplus
extern "C" {
#endif

#define PSTORE_DIR "/sys/fs/pstore"

/**
 * Processes the pstore directory.
 * At the moment, all it does is delete all files and symlinks inside the pstore_dir.
 * The intention is to add additional logic over time, similar to systemd-pstore and more.
 * @param pstore_dir The path of the pstore directory. Normally, PSTORE_DIR is used.
 * For unit testing purposes, another path can be given.
 */
void memfault_reboot_process_pstore_files(char *pstore_dir);

#ifdef __cplusplus
}
#endif
