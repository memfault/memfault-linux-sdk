//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiting of coredumps.

#include "coredump_ratelimiter.h"

#include <stdlib.h>

#define RATE_LIMIT_FILENAME "coredump_rate_limit"

/**
 * @brief Checks the coredump rate limiter.
 *
 * @param ratelimiter_filename Rate limiter filename
 * @return true if the coredump should be processed, false otherwise
 */
bool coredump_check_rate_limiter(const char *ratelimiter_filename, int rate_limit_count,
                                 int rate_limit_duration_seconds) {
  sMemfaultdRateLimiter *const rate_limiter = memfaultd_rate_limiter_init(
    rate_limit_count, rate_limit_duration_seconds, ratelimiter_filename);
  const bool result = memfaultd_rate_limiter_check_event(rate_limiter);
  memfaultd_rate_limiter_destroy(rate_limiter);
  return result;
}
