//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Rate limiting of coredumps.

#include "coredump_ratelimiter.h"

#include <stdlib.h>

#include "memfault/util/config.h"

#define RATE_LIMIT_FILENAME "coredump_rate_limit"

/**
 * @brief Create a new coredump rate limiter.
 *
 * @param config Config handle
 * @return sMemfaultdRateLimiter an initialized rate limiter
 */
sMemfaultdRateLimiter *coredump_create_rate_limiter(sMemfaultdConfig *config) {
  //! Initialise the corefile rate limiter, errors here aren't critical
  int rate_limit_count = 0;
  int rate_limit_duration_seconds = 0;

  bool dev_mode = false;
  memfaultd_config_get_boolean(config, NULL, CONFIG_KEY_DEV_MODE, &dev_mode);

  if (!dev_mode) {
    memfaultd_config_get_integer(config, "coredump_plugin", "rate_limit_count", &rate_limit_count);
    memfaultd_config_get_integer(config, "coredump_plugin", "rate_limit_duration_seconds",
                                 &rate_limit_duration_seconds);
  }

  char *ratelimiter_filename = memfaultd_config_generate_tmp_filename(config, RATE_LIMIT_FILENAME);
  if (!ratelimiter_filename) {
    return NULL;
  }

  sMemfaultdRateLimiter *rate_limiter = memfaultd_rate_limiter_init(
    rate_limit_count, rate_limit_duration_seconds, ratelimiter_filename);
  free(ratelimiter_filename);

  return rate_limiter;
}
