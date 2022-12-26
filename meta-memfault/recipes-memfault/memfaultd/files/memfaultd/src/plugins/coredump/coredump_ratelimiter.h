//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiting of coredumps.

#pragma once

#include "memfault/util/rate_limiter.h"
#include "memfaultd.h"

#ifdef __cplusplus
extern "C" {
#endif

sMemfaultdRateLimiter *coredump_create_rate_limiter(sMemfaultd *memfaultd);

#ifdef __cplusplus
}
#endif
