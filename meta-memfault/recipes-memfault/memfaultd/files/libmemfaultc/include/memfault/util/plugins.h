#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Definition for plugins entrypoints.

#include "memfaultd.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
  memfaultd_plugin_init init;
  sMemfaultdPluginCallbackFns *fns;
  const char name[32];
  const char ipc_name[32];
} sMemfaultdPluginDef;

#define PLUGIN_ATTRIBUTES_IPC_NAME "ATTRIBUTES"
bool memfaultd_attributes_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);

#ifdef PLUGIN_REBOOT
bool memfaultd_reboot_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
void memfaultd_reboot_data_collection_enabled(sMemfaultd *memfaultd, bool data_collection_enabled);
#endif
#ifdef PLUGIN_SWUPDATE
bool memfaultd_swupdate_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif
#ifdef PLUGIN_COLLECTD
  #define PLUGIN_COLLECTD_IPC_NAME "COLLECTD"
bool memfaultd_collectd_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif
#ifdef PLUGIN_COREDUMP
bool memfaultd_coredump_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif

/**
 * @brief List of all enabled plugins.
 */
extern sMemfaultdPluginDef g_plugins[];

/**
 * @brief Count of enabled plugins.
 */
extern const unsigned long int g_plugins_count;

/**
 * @brief Call the init function of all defined plugins
 *
 * @param handle Main memfaultd handle
 */
void memfaultd_load_plugins(sMemfaultd *handle);

/**
 * @brief Call the destroy function of all defined g_plugins
 */
void memfaultd_destroy_plugins(void);

/**
 * @brief Search for a plugin to process this IPC message and delegate processing.
 */
bool memfaultd_plugins_process_ipc(struct msghdr *msg, size_t received_size);

#ifdef __cplusplus
}
#endif
