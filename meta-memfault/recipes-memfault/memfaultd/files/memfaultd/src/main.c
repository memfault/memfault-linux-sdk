//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Main file for memfault linux SDK.
//!
//! @details
//! We build one binary on disk and create two links to it (memfaultd and memfaultctl).
//! This approach is inspired by the busybox project.

#include <libgen.h>
#include <stdio.h>
#include <string.h>

#include "memfaultctl/memfaultctl.h"
#include "memfaultd.h"

int main(int argc, char **argv) {
  char *cmd_name = basename(argv[0]);
  if (strcmp(cmd_name, "memfaultd") == 0) {
    return memfaultd_main(argc, argv);
  } else {
    return memfaultctl_main(argc, argv);
  }
}
