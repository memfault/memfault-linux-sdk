//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Change config options at runtime.

#include "memfault/util/runtime_config.h"

#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <unistd.h>

#include "memfault/util/systemd.h"

int memfault_set_runtime_bool_and_reload(sMemfaultdConfig *config, const char *config_key,
                                         const char *description, bool value) {
  bool current_state = false;

  if (memfaultd_config_get_boolean(config, "", config_key, &current_state) &&
      current_state == value) {
    char *uppercase_description = strdup(description);
    uppercase_description[0] = toupper(uppercase_description[0]);
    printf("%s is already %s.\n", uppercase_description, value ? "enabled" : "disabled");
    free(uppercase_description);
    return 0;
  }
  printf("%s %s.\n", value ? "Enabling" : "Disabling", description);
  memfaultd_config_set_boolean(config, "", config_key, value);

  // Restart memfaultd
  if (getuid() != 0) {
    printf("Not running as root. Will not attempt to restart memfaultd.\n");
    return 0;
  } else if (!memfaultd_restart_service_if_running("memfaultd.service")) {
    // Error message printed by memfaultd_restart_service_if_running
    return -1;
  }
  return 0;
}
