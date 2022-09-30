//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Disk utilities

#define _XOPEN_SOURCE 800
#include <ftw.h>
#include <stdbool.h>
#include <sys/vfs.h>

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
