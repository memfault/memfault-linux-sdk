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

typedef enum MemfaultdHttpMethod {
  kMemfaultdHttpMethod_POST,
  kMemfaultdHttpMethod_PATCH
} eMemfaultdHttpMethod;

typedef enum MemfaultdNetworkResult {
  /**
   * The network operation was successful.
   */
  kMemfaultdNetworkResult_OK,
  /**
   * The network operation was not successful, but retrying later is sensible because the error is
   * likely to be transient.
   */
  kMemfaultdNetworkResult_ErrorRetryLater,
  /**
   * The network operation was not successful and retrying is not sensible because the error is
   * not transient.
   */
  kMemfaultdNetworkResult_ErrorNoRetry,
} eMemfaultdNetworkResult;

typedef struct MemfaultdNetwork sMemfaultdNetwork;

sMemfaultdNetwork *memfaultd_network_init(sMemfaultd *memfaultd);
void memfaultd_network_destroy(sMemfaultdNetwork *handle);
eMemfaultdNetworkResult memfaultd_network_post(sMemfaultdNetwork *handle, const char *endpoint,
                                               eMemfaultdHttpMethod method, const char *payload,
                                               char **data, size_t *len);

eMemfaultdNetworkResult memfaultd_network_file_upload(sMemfaultdNetwork *handle,
                                                      const char *commit_endpoint,
                                                      const char *payload, bool is_gzipped);

#endif
