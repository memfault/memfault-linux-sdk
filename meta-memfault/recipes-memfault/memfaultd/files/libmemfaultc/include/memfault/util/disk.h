#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Disk utilities

#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

size_t memfaultd_get_free_space(const char* path, bool privileged);
size_t memfaultd_get_folder_size(const char* path);

typedef struct MemfaultStorageQuota {
  size_t min_headroom;
  size_t max_usage;
  size_t max_size;
} sMemfaultStorageQuota;

size_t memfaultd_calculate_available_space(const char* dir_path,
                                           const sMemfaultStorageQuota* quota);

#ifdef __cplusplus
}
#endif
