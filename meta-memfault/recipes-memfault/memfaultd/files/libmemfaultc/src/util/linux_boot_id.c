//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Utility function to read the current Linux boot ID.

#include "memfault/util/linux_boot_id.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

bool memfault_linux_boot_id_read(char boot_id[UUID_STR_LEN]) {
  bool result = false;

  const int fd = open("/proc/sys/kernel/random/boot_id", O_RDONLY);
  if (fd == -1) {
    fprintf(stderr, "linux_boot_id:: failed to open: %s\n", strerror(errno));
    goto cleanup;
  }
  const ssize_t sz = read(fd, boot_id, UUID_STR_LEN - 1);
  if (sz == -1) {
    fprintf(stderr, "linux_boot_id:: failed to read: %s\n", strerror(errno));
    goto cleanup;
  } else if (sz != UUID_STR_LEN - 1) {
    boot_id[0] = '\0';
    fprintf(stderr, "linux_boot_id:: not enough bytes read (%d)\n", (int)sz);
    goto cleanup;
  }

  boot_id[UUID_STR_LEN - 1] = '\0';
  result = true;

cleanup:
  if (fd != -1) {
    close(fd);
  }
  return result;
}
