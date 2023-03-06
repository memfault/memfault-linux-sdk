//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include "memfault/util/plugins.h"

#include <stdio.h>
#include <string.h>

#include "memfault/core/math.h"

sMemfaultdPluginDef g_plugins[] = {
  {.name = "attributes", .init = memfaultd_attributes_init, .ipc_name = "ATTRIBUTES"},
#ifdef PLUGIN_REBOOT
  {.name = "reboot", .init = memfaultd_reboot_init},
#endif
#ifdef PLUGIN_SWUPDATE
  {.name = "swupdate", .init = memfaultd_swupdate_init},
#endif
#ifdef PLUGIN_COLLECTD
  {.name = "collectd", .init = memfaultd_collectd_init, .ipc_name = PLUGIN_COLLECTD_IPC_NAME},
#endif
#ifdef PLUGIN_COREDUMP
  {.name = "coredump", .init = memfaultd_coredump_init, .ipc_name = "CORE"},
#endif
#ifdef PLUGIN_LOGGING
  {.name = "logging"},
#endif
};

const unsigned long int g_plugins_count = MEMFAULT_ARRAY_SIZE(g_plugins);

void memfaultd_load_plugins(sMemfaultd *handle) {
  for (unsigned int i = 0; i < g_plugins_count; ++i) {
    if (g_plugins[i].init != NULL && !g_plugins[i].init(handle, &g_plugins[i].fns)) {
      fprintf(stderr, "memfaultd:: Failed to initialize %s plugin, destroying.\n",
              g_plugins[i].name);
      g_plugins[i].fns = NULL;
    }
  }
}

void memfaultd_destroy_plugins(void) {
  for (unsigned int i = 0; i < g_plugins_count; ++i) {
    if (g_plugins[i].fns != NULL && g_plugins[i].fns->plugin_destroy) {
      g_plugins[i].fns->plugin_destroy(g_plugins[i].fns->handle);
    }
  }
}

bool memfaultd_plugins_process_ipc(struct msghdr *msg, size_t received_size) {
  for (unsigned int i = 0; i < g_plugins_count; ++i) {
    if (g_plugins[i].ipc_name[0] == '\0' || !g_plugins[i].fns ||
        !g_plugins[i].fns->plugin_ipc_msg_handler) {
      // Plugin doesn't process IPC messages
      continue;
    }

    if (received_size <= strlen(g_plugins[i].ipc_name) ||
        strcmp(g_plugins[i].ipc_name, msg->msg_iov[0].iov_base) != 0) {
      // Plugin doesn't match IPC signature
      continue;
    }

    if (!g_plugins[i].fns->plugin_ipc_msg_handler(g_plugins[i].fns->handle, msg, received_size)) {
      fprintf(stderr, "memfaultd:: Plugin %s failed to process IPC message.\n", g_plugins[i].name);
    }
    return true;
  }
  return false;
}
