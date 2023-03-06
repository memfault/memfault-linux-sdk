//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiting of coredumps.

#pragma once

#include "memfault/util/config.h"
#include "memfault/util/rate_limiter.h"

#ifdef __cplusplus
extern "C" {
#endif

sMemfaultdRateLimiter *coredump_create_rate_limiter(sMemfaultdConfig *config);

#ifdef __cplusplus
}
#endif
