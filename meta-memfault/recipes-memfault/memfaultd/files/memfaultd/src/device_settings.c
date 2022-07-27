//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd device settings implementation

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "memfaultd.h"

#define DEFAULT_BASE_URL "https://device.memfault.com"

#define INFO_BINARY "memfault-device-info"

/**
 * @brief Destroy the device settings object
 *
 * @param handle device settings object
 */
void memfaultd_device_settings_destroy(sMemfaultdDeviceSettings *handle) {
  if (handle) {
    if (handle->device_id) {
      free(handle->device_id);
    }
    if (handle->hardware_version) {
      free(handle->hardware_version);
    }

    free(handle);
  }
}

/**
 * @brief Initialise the device settings object
 *
 * @return memfaultd_device_settings_t* device settings object
 */
sMemfaultdDeviceSettings *memfaultd_device_settings_init(void) {
  FILE *fd = popen(INFO_BINARY, "r");
  if (!fd) {
    fprintf(stderr, "device_settings:: Unable to execute '%s'\n", INFO_BINARY);
    return NULL;
  }

  sMemfaultdDeviceSettings *handle = calloc(sizeof(sMemfaultdDeviceSettings), 1);

  char line[1024];
  while (fgets(line, sizeof(line), fd)) {
    char *name = strtok(line, "=");
    char *val = strtok(NULL, "\n");

    if (strcmp(name, "MEMFAULT_DEVICE_ID") == 0) {
      handle->device_id = strdup(val);
    } else if (strcmp(name, "MEMFAULT_HARDWARE_VERSION") == 0) {
      handle->hardware_version = strdup(val);
    } else {
      fprintf(stderr, "device_settings:: Unknown option in %s : '%s'\n", INFO_BINARY, name);
    }
  }

  pclose(fd);

  bool failed = false;
  if (!handle->device_id) {
    fprintf(stderr, "device_settings:: MEMFAULT_DEVICE_ID not set in %s\n", INFO_BINARY);
    failed = true;
  }
  if (!handle->hardware_version) {
    fprintf(stderr, "device_settings:: MEMFAULT_HARDWARE_VERSION not set in %s\n", INFO_BINARY);
    failed = true;
  }

  if (failed) {
    memfaultd_device_settings_destroy(handle);
    return NULL;
  }

  return handle;
}
