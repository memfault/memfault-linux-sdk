//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiting of coredumps.

#pragma once

#include "memfault/util/rate_limiter.h"

#ifdef __cplusplus
extern "C" {
#endif

bool coredump_check_rate_limiter(const char *ratelimiter_filename, int rate_limit_count,
                                 int rate_limit_duration_seconds);

#ifdef __cplusplus
}
#endif
