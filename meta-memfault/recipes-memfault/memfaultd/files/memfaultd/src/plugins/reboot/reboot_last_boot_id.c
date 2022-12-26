//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Utility to persist the boot id of the last tracked reboot to a file.

#include "reboot_last_boot_id.h"

#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "memfault/util/linux_boot_id.h"

static bool prv_read_tracked_boot_id(FILE *fd, const char *last_tracked_boot_id_file,
                                     char tracked_boot_id[UUID_STR_LEN]) {
  if (fseek(fd, 0, SEEK_SET) != 0) {
    fprintf(stderr, "reboot:: fseek failed '%s', %s.\n", last_tracked_boot_id_file,
            strerror(errno));
    tracked_boot_id[0] = '\0';
    return false;
  }

  const size_t sz = fread(tracked_boot_id, 1, UUID_STR_LEN - 1, fd);
  if (sz != UUID_STR_LEN - 1) {
    fprintf(stderr, "reboot:: read (%u) %s.\n", (unsigned int)sz, tracked_boot_id);
    tracked_boot_id[0] = '\0';
    return false;
  }
  tracked_boot_id[UUID_STR_LEN - 1] = '\0';
  return true;
}

static bool prv_write_current_boot_id(FILE *fd, const char *last_tracked_boot_id_file,
                                      const char *current_boot_id) {
  if (ftruncate(fileno(fd), 0) != 0) {
    fprintf(stderr, "reboot:: ftruncate failed '%s', %s.\n", last_tracked_boot_id_file,
            strerror(errno));
    return false;
  }

  if (fseek(fd, 0, SEEK_SET) != 0) {
    fprintf(stderr, "reboot:: fseek failed '%s', %s.\n", last_tracked_boot_id_file,
            strerror(errno));
    return false;
  }

  if (fputs(current_boot_id, fd) < 0) {
    fprintf(stderr, "reboot:: fputs failed '%s', %s.\n", last_tracked_boot_id_file,
            strerror(errno));
    return false;
  }

  return true;
}

bool memfault_reboot_is_untracked_boot_id(const char *last_tracked_boot_id_file,
                                          const char *current_boot_id) {
  bool result = false;

  FILE *fd = fopen(last_tracked_boot_id_file, "a+");
  if (fd == NULL) {
    fprintf(stderr, "reboot:: Failed to open %s\n", last_tracked_boot_id_file);
    goto cleanup;
  }

  char tracked_boot_id[UUID_STR_LEN] = {0};
  if (!prv_read_tracked_boot_id(fd, last_tracked_boot_id_file, tracked_boot_id)) {
    // Note: it's possible the file contained a malformed UUID.
    // In this case, let's auto-heal and continue as if there was no UUID found.
    fprintf(stderr, "reboot:: no last tracked boot_id found\n");
  }

  if (strcmp(tracked_boot_id, current_boot_id) == 0) {
    fprintf(stderr, "reboot:: boot_id already tracked (%s)!\n", current_boot_id);
    goto cleanup;
  }

  if (!prv_write_current_boot_id(fd, last_tracked_boot_id_file, current_boot_id)) {
    goto cleanup;
  }

  result = true;

cleanup:
  if (fd != NULL) {
    fclose(fd);
  }
  return result;
}
