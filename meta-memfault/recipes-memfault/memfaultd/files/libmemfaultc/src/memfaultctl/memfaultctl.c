//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultctl implementation

#include "memfaultctl.h"

#include <errno.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "crash.h"
#include "memfault/util/config.h"
#include "memfault/util/device_settings.h"
#include "memfault/util/ipc.h"
#include "memfault/util/plugins.h"
#include "memfault/util/reboot_reason.h"

typedef struct MemfaultCtl {
  char *config_file;
  sMemfaultdConfig *config;
} sMemfaultCtl;

static sMemfaultCtl prv_init_handle(char *config_file) {
  sMemfaultCtl handle = {.config_file = CONFIG_FILE};

  if (config_file) {
    handle.config_file = config_file;
  }

  handle.config = memfaultd_config_init(handle.config_file);
  if (!handle.config) {
    exit(-1);
  }

  return handle;
}

static void prv_deinit_handle(sMemfaultCtl *handle) {
  if (handle->config) {
    memfaultd_config_destroy(handle->config);
  }
}

int cmd_reboot(char *config_file, int reboot_reason_arg) {
  sMemfaultCtl handle = prv_init_handle(config_file);

  const char *reboot_reason_file;

  if (!memfaultd_config_get_string(handle.config, "reboot_plugin", "last_reboot_reason_file",
                                   &reboot_reason_file)) {
    fprintf(stderr, "Unable to read location of reboot_reason_file in configuration.\n");
    prv_deinit_handle(&handle);
    return -1;
  }

  eMemfaultRebootReason reboot_reason = (eMemfaultRebootReason)reboot_reason_arg;

  if (!memfaultd_is_reboot_reason_valid(reboot_reason)) {
    fprintf(stderr,
            "Invalid reboot reason '%d'.\n"
            "Refer to https://docs.memfault.com/docs/platform/reference-reboot-reason-ids\n",
            reboot_reason);
    prv_deinit_handle(&handle);
    return -1;
  }

  printf("Rebooting with reason %d (%s)\n", reboot_reason,
         memfaultd_reboot_reason_str(reboot_reason));
  FILE *file = fopen(reboot_reason_file, "w");

  prv_deinit_handle(&handle);

  if (!file) {
    fprintf(stderr, "Unable to open file: %s\n", strerror(errno));
    return -1;
  }
  if (fprintf(file, "%d", reboot_reason) < 0) {
    fprintf(stderr, "Error writing reboot reason: %s\n", strerror(errno));
    fclose(file);
    return -1;
  }
  fclose(file);

  if (system("reboot") < 0) {
    fprintf(stderr, "Unable to call 'reboot': %s\n", strerror(errno));
    return -1;
  }
  return 0;
}

int cmd_request_metrics(void) {
#ifdef PLUGIN_COLLECTD
  return memfaultd_ipc_sendmsg((uint8_t *)PLUGIN_COLLECTD_IPC_NAME,
                               sizeof(PLUGIN_COLLECTD_IPC_NAME))
           ? 0
           : -1;
#endif
  return 0;
}
