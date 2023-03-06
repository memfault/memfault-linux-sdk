//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Disk utilities

#define _XOPEN_SOURCE 800
#include "memfault/util/disk.h"

#include <ftw.h>
#include <stdbool.h>
#if __linux__
  #include <sys/vfs.h>
#else
  #define _DARWIN_C_SOURCE 1
  #include <sys/mount.h>
  #include <sys/param.h>
#endif

#include "memfault/core/math.h"

size_t memfaultd_get_free_space(const char* path, bool privileged) {
  struct statfs buf;
  if (statfs(path, &buf) == -1) {
    return 0;
  }

  if (privileged) {
    return buf.f_bsize * buf.f_bfree;
  } else {
    return buf.f_bsize * buf.f_bavail;
  }
}

static size_t s_foldersize;

static int prv_get_folder_size_sum(const char* fpath, const struct stat* sb, int typeflag,
                                   struct FTW* ftwbuf) {
  (void)fpath;
  (void)typeflag;
  (void)ftwbuf;
  s_foldersize += sb->st_size;
  return 0;
}

size_t memfaultd_get_folder_size(const char* path) {
  s_foldersize = 0;
  if (nftw(path, &prv_get_folder_size_sum, 1, FTW_PHYS | FTW_MOUNT) == -1) {
    return 0;
  }
  return s_foldersize;
}

size_t memfaultd_calculate_available_space(const char* dir_path,
                                           const sMemfaultStorageQuota* quota) {
  if (quota->min_headroom == 0 && quota->max_usage == 0 && quota->max_size == 0) {
    //! No limits, return non-privileged space left on device - leaves 5% reserve on ext[2-4]
    //! filesystems
    const bool privileged = false;
    return memfaultd_get_free_space(dir_path, privileged);
  }

  size_t headroom_delta = ~0;
  if (quota->min_headroom != 0) {
    const bool privileged = true;
    const size_t free = memfaultd_get_free_space(dir_path, privileged);
    if (free <= quota->min_headroom) {
      return 0;
    }
    headroom_delta = free - quota->min_headroom;
  }

  size_t usage_delta = ~0;
  if (quota->max_usage != 0) {
    const size_t used = memfaultd_get_folder_size(dir_path);
    if (used >= quota->max_usage) {
      return 0;
    }
    usage_delta = quota->max_usage - used;
  }

  return MEMFAULT_MIN(MEMFAULT_MIN(headroom_delta, usage_delta), quota->max_size);
}
