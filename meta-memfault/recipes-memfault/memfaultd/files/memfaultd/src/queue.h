//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! File-backed transmit queue management definition
//!

#ifndef __MEMFAULT_QUEUE_H
#define __MEMFAULT_QUEUE_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

#include "memfaultd.h"

typedef struct MemfaultdQueue sMemfaultdQueue;

sMemfaultdQueue *memfaultd_queue_init(sMemfaultd *memfaultd, int size);
void memfaultd_queue_destroy(sMemfaultdQueue *handle);
void memfaultd_queue_reset(sMemfaultdQueue *handle);
bool memfaultd_queue_write(sMemfaultdQueue *handle, const uint8_t *payload,
                           uint32_t payload_size_bytes);
uint8_t *memfaultd_queue_read_head(sMemfaultdQueue *handle, uint32_t *payload_size_bytes);
bool memfaultd_queue_complete_read(sMemfaultdQueue *handle);

#ifdef __cplusplus
}
#endif
#endif
