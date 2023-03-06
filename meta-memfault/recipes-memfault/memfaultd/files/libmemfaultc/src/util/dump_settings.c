//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Print the current configuration, runtime settings and build time settings to the console.

#include <stdio.h>
#include <string.h>

#include "memfault/util/config.h"
#include "memfault/util/device_settings.h"
#include "memfault/util/plugins.h"
#include "memfault/util/version.h"
#include "memfaultd.h"

void memfaultd_dump_settings(sMemfaultdDeviceSettings *settings, sMemfaultdConfig *config,
                             const char *config_file) {
  memfaultd_config_dump_config(config, config_file);

  if (settings) {
    printf("Device configuration from memfault-device-info:\n");
    printf("  MEMFAULT_DEVICE_ID=%s\n", settings->device_id);
    printf("  MEMFAULT_HARDWARE_VERSION=%s\n", settings->hardware_version);
  } else {
    printf("Device configuration from memfault-device-info: IS NOT AVAILABLE.\n");
  }
  printf("\n");

  memfault_version_print_info();
  printf("\n");

  printf("Plugin enabled:\n");
  for (unsigned int i = 0; i < g_plugins_count; ++i) {
    if (g_plugins[i].name[0] != '\0') {
      printf("  %s\n", g_plugins[i].name);
    }
  }
  printf("\n");
}
