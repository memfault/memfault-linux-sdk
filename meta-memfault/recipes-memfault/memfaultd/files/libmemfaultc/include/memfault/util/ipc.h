#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Interact via IPC with memfaultd.

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <time.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MEMFAULTD_IPC_SOCKET_PATH "/tmp/memfault-ipc.sock"

/**
 * Send a SIGUSR1 signal to memfaultd to immediately process the queue.
 */
bool memfaultd_send_flush_queue_signal(void);

/**
 * Send an IPC message to memfaultd.
 *
 * The first bytes of the message should be the ipc_plugin_name of the plugin to process the
 * message.
 */
bool memfaultd_ipc_sendmsg(uint8_t *msg, size_t len);

typedef struct MemfaultAttributesIPC {
  char name[11] /*"ATTRIBUTES\0" */;
  time_t timestamp;
  char json[];
} sMemfaultAttributesIPC;

#ifdef __cplusplus
}
#endif
