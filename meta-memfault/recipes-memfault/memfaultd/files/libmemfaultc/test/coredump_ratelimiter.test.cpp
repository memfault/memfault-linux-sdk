//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for coredump_ratelimiter.c
//!

#include "memfault-core-handler/coredump_ratelimiter.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>
#include <stdlib.h>
#include <string.h>

TEST_GROUP(TestCoreDumpRateLimiterGroup){};

extern "C" {

typedef struct MemfaultdRateLimiter {
  int count = -1;
  int duration = -1;
} sMemfaultdRateLimiter;

sMemfaultdRateLimiter *memfaultd_rate_limiter_init(int count, int duration, const char *filename) {
  sMemfaultdRateLimiter *r = (sMemfaultdRateLimiter *)calloc(sizeof(sMemfaultdRateLimiter), 1);
  r->count = count;
  r->duration = duration;

  return r;
}

bool memfaultd_config_get_integer(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  int *val) {
  return mock()
    .actualCall("memfaultd_config_get_integer")
    .withStringParameter("parent_key", parent_key)
    .withStringParameter("key", key)
    .withOutputParameter("val", val)
    .returnBoolValue();
}

bool memfaultd_config_get_boolean(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  bool *val) {
  return mock()
    .actualCall("memfaultd_config_get_boolean")
    .withStringParameter("parent_key", parent_key)
    .withStringParameter("key", key)
    .withOutputParameter("val", val)
    .returnBoolValue();
}

char *memfaultd_config_generate_tmp_filename(sMemfaultdConfig *handle, const char *filename) {
  return (char *)mock()
    .actualCall("memfaultd_config_generate_tmp_filename")
    .withStringParameter("filename", filename)
    .returnStringValue();
}
}

/**
 * @brief When memfaultd is not in dev mode
 *
 */
TEST(TestCoreDumpRateLimiterGroup, NormalMode) {
  int rate_limit_count = 42;
  int rate_limit_duration = 60;
  bool dev_mode = false;

  mock()
    .expectOneCall("memfaultd_config_get_boolean")
    .withStringParameter("parent_key", NULL)
    .withStringParameter("key", "enable_dev_mode")
    .withOutputParameterReturning("val", &dev_mode, sizeof(dev_mode))
    .andReturnValue(true);
  mock()
    .expectOneCall("memfaultd_config_get_integer")
    .withStringParameter("parent_key", "coredump_plugin")
    .withStringParameter("key", "rate_limit_count")
    .withOutputParameterReturning("val", &rate_limit_count, sizeof(rate_limit_count))
    .andReturnValue(true);
  mock()
    .expectOneCall("memfaultd_config_get_integer")
    .withStringParameter("parent_key", "coredump_plugin")
    .withStringParameter("key", "rate_limit_duration_seconds")
    .withOutputParameterReturning("val", &rate_limit_duration, sizeof(rate_limit_duration))
    .andReturnValue(true);
  mock()
    .expectOneCall("memfaultd_config_generate_tmp_filename")
    .withStringParameter("filename", "coredump_rate_limit")
    .andReturnValue(strdup("coredump_rate_limit"));

  auto r = coredump_create_rate_limiter(NULL);

  mock().checkExpectations();
  CHECK(r->count == rate_limit_count);
  CHECK(r->duration == rate_limit_duration);

  mock().clear();
  free(r);
}

/**
 * @brief When memfaultd is in dev mode
 *
 */
TEST(TestCoreDumpRateLimiterGroup, DevMode) {
  bool dev_mode = true;

  mock()
    .expectOneCall("memfaultd_config_get_boolean")
    .withStringParameter("parent_key", NULL)
    .withStringParameter("key", "enable_dev_mode")
    .withOutputParameterReturning("val", &dev_mode, sizeof(dev_mode))
    .andReturnValue(true);
  mock()
    .expectOneCall("memfaultd_config_generate_tmp_filename")
    .withStringParameter("filename", "coredump_rate_limit")
    .andReturnValue(strdup("coredump_rate_limit"));

  auto r = coredump_create_rate_limiter(NULL);

  mock().checkExpectations();
  mock().clear();

  CHECK(r->count == 0);
  CHECK(r->duration == 0);
  free(r);
}
