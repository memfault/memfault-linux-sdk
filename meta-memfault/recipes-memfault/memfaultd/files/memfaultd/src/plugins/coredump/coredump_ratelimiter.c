//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiting of coredumps.

#include "coredump_ratelimiter.h"

#define RATE_LIMIT_FILENAME "coredump_rate_limit"

/**
 * @brief Create a new coredump rate limiter.
 *
 * @param memfaultd Main memfaultd handle
 * @return sMemfaultdRateLimiter an initialized rate limiter
 */
sMemfaultdRateLimiter *coredump_create_rate_limiter(sMemfaultd *memfaultd) {
  //! Initialise the corefile rate limiter, errors here aren't critical
  int rate_limit_count = 0;
  int rate_limit_duration_seconds = 0;
  if (!memfaultd_is_dev_mode(memfaultd)) {
    memfaultd_get_integer(memfaultd, "coredump_plugin", "rate_limit_count", &rate_limit_count);
    memfaultd_get_integer(memfaultd, "coredump_plugin", "rate_limit_duration_seconds",
                          &rate_limit_duration_seconds);
  }

  return memfaultd_rate_limiter_init(memfaultd, rate_limit_count, rate_limit_duration_seconds,
                                     RATE_LIMIT_FILENAME);
}
