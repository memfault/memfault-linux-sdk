//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include "memfault/util/pid.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

/**
 * @brief Checks for memfaultd daemon PID file
 *
 * @return true PID file exists
 * @return false Does not exist
 */
bool memfaultd_check_for_pid_file(void) {
  const int fd = open(PID_FILE, O_WRONLY | O_EXCL, S_IRUSR | S_IWUSR);
  if (fd == -1) {
    if (errno == ENOENT) {
      return false;
    } else {
      // PID file exists, but can't open it for some reason
      return true;
    }
  }

  close(fd);
  return true;
}

/**
 * @brief Returns memfaultd PID
 * @return memfaultd PID or -1.
 */
pid_t memfaultd_get_pid(void) {
  pid_t pid = -1;
  char buf[12];

  const int fd = open(PID_FILE, O_RDONLY);

  if (fd < 0) {
    return pid;
  }

  ssize_t len = read(fd, buf, sizeof(buf) - 1);
  if (len < 0) {
    goto cleanup;
  }
  buf[len] = 0;
  pid = atoi(buf);

cleanup:
  close(fd);
  return pid;
}
