#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiter library functions

#include <stdbool.h>

#include "memfaultd.h"

typedef struct MemfaultdRateLimiter sMemfaultdRateLimiter;

#ifdef __cplusplus
extern "C" {
#endif

sMemfaultdRateLimiter *memfaultd_rate_limiter_init(sMemfaultd *memfaultd, const int count,
                                                   const int duration, const char *filename);
bool memfaultd_rate_limiter_check_event(sMemfaultdRateLimiter *handle);
void memfaultd_rate_limiter_destroy(sMemfaultdRateLimiter *handle);

#ifdef __cplusplus
}
#endif
