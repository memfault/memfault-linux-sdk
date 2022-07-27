//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Network POST & GET API wrapper around libCURL
//!

#ifndef __MEMFAULT_NETWORK_H
#define __MEMFAULT_NETWORK_H

#include <stdbool.h>
#include <stddef.h>

#include "memfaultd.h"

typedef struct MemfaultdNetwork sMemfaultdNetwork;

sMemfaultdNetwork *memfaultd_network_init(sMemfaultd *memfaultd);
void memfaultd_network_destroy(sMemfaultdNetwork *handle);
bool memfaultd_network_post(sMemfaultdNetwork *handle, const char *endpoint, const char *payload,
                            char **data, size_t *len);
bool memfaultd_network_get(sMemfaultdNetwork *handle, const char *endpoint, char **data,
                           size_t *len);

#endif
