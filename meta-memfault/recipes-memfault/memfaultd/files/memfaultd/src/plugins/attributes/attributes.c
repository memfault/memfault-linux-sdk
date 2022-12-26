//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfault attributes plugin implementation

#include <errno.h>
#include <json-c/json.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <systemd/sd-bus.h>
#include <unistd.h>

#include "memfault/core/math.h"
#include "memfault/util/ipc.h"
#include "memfault/util/string.h"
#include "memfaultd.h"

struct MemfaultdPlugin {
  sMemfaultd *memfaultd;
};

static sMemfaultdTxData *prv_build_queue_entry(sMemfaultAttributesIPC *msg,
                                               uint32_t *payload_size) {
  size_t json_len = strlen(msg->json);
  *payload_size = sizeof(time_t) + json_len + 1;

  sMemfaultdTxDataAttributes *data;
  if (!(data = malloc(sizeof(sMemfaultdTxDataAttributes) + *payload_size))) {
    fprintf(stderr, "network:: Failed to create upload_request buffer\n");
    return NULL;
  }

  data->type = kMemfaultdTxDataType_Attributes;
  data->timestamp = msg->timestamp;
  strcpy((char *)data->json, msg->json);

  return (sMemfaultdTxData *)data;
}

/**
 * Build a queue entry for memfaultd.
 */
static bool prv_msg_handler(sMemfaultdPlugin *handle, struct msghdr *msghdr, size_t received_size) {
  int ret = EXIT_SUCCESS;

  sMemfaultAttributesIPC *msg = msghdr->msg_iov[0].iov_base;

  // Transform JSON Array into a Queue message
  uint32_t len = 0;
  struct MemfaultdTxData *data = prv_build_queue_entry(msg, &len);

  if (!memfaultd_txdata(handle->memfaultd, data, len)) {
    ret = EXIT_FAILURE;
    goto cleanup;
  }

cleanup:
  free(data);
  return ret == EXIT_SUCCESS;
}

static sMemfaultdPluginCallbackFns s_fns = {.plugin_ipc_msg_handler = prv_msg_handler};

/**
 * @brief Initialises attributes plugin
 *
 * @param memfaultd Main memfaultd handle
 * @return callbackFunctions_t Plugin function table
 */
bool memfaultd_attributes_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns) {
  sMemfaultdPlugin *handle = calloc(sizeof(sMemfaultdPlugin), 1);
  if (!handle) {
    fprintf(stderr, "attributes:: Failed to allocate plugin handle\n");
    return false;
  }

  handle->memfaultd = memfaultd;
  *fns = &s_fns;
  (*fns)->handle = handle;

  return true;
}
