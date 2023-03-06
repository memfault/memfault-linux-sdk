//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! coredump plugin implementation

#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "memfault/util/disk.h"
#include "memfaultd.h"

#define CORE_PATTERN_PATH "/proc/sys/kernel/core_pattern"
#define CORE_PATTERN_FMT "|/usr/sbin/memfault-core-handler -c %s %%P"

/**
 * @brief Initialises coredump plugin
 *
 * @param memfaultd Main memfaultd handle
 * @return callbackFunctions_t Plugin function table
 */
bool memfaultd_coredump_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns) {
  *fns = NULL;

  bool enable_data_collection = false;
  if (!memfaultd_get_boolean(memfaultd, "", "enable_data_collection", &enable_data_collection) ||
      !enable_data_collection) {
    //! Even though comms are disabled, we still want to log any crashes which have happened
    fprintf(stderr, "coredump:: Data collection is off, plugin disabled.\n");
  }

  // Write core_patten to kernel
  int fd;
  if ((fd = open(CORE_PATTERN_PATH, O_WRONLY, 0)) == -1) {
    fprintf(stderr, "coredump:: Failed to open kernel core pattern file : %s\n", strerror(errno));
    goto cleanup;
  }
  if (dprintf(fd, CORE_PATTERN_FMT, memfaultd_get_config_file(memfaultd)) < 0) {
    fprintf(stderr, "coredump:: Failed to write kernel core pattern : %s\n", strerror(errno));
    goto cleanup;
  }
  close(fd);
  return true;

cleanup:
  if (fd != -1) {
    close(fd);
  }
  return false;
}
