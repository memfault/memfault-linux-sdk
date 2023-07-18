//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for rate_limiter.c
//!

#include "memfault/util/rate_limiter.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>
#include <sys/time.h>
#include <unistd.h>

#include <cstring>
#include <fstream>
#include <iostream>
#include <sstream>

extern "C" {
time_t *memfaultd_rate_limiter_get_history(sMemfaultdRateLimiter *handle);
}

TEST_BASE(MemfaultdRateLimiterUtest) {
  char tmp_dir[32] = {0};
  char tmp_rate_limit_file[64] = {0};
  struct timeval tv = {0};

  void setup() override {
    strcpy(tmp_dir, "/tmp/memfaultd.XXXXXX");
    mkdtemp(tmp_dir);
    snprintf(tmp_rate_limit_file, sizeof(tmp_rate_limit_file), "%s/ratelimit", tmp_dir);
  }

  void teardown() override {
    unlink(tmp_rate_limit_file);
    rmdir(tmp_dir);
    mock().checkExpectations();
    mock().clear();
  }

  void write_ratelimit_file(const char *val) {
    std::ofstream fd(tmp_rate_limit_file);
    fd << val;
  }

  char *read_ratelimit_file() {
    std::ifstream fd(tmp_rate_limit_file);
    std::stringstream buf;
    buf << fd.rdbuf();
    return strdup(buf.str().c_str());
  }
};

TEST_GROUP_BASE(TestGroup_Init, MemfaultdRateLimiterUtest){};

TEST(TestGroup_Init, InitFailures) {
  CHECK(!memfaultd_rate_limiter_init(0, 3600, NULL));  //! count is 0
  CHECK(!memfaultd_rate_limiter_init(5, 0, NULL));     //! duration is 0
}

TEST(TestGroup_Init, InitSuccessNoHistoryFile) {
  sMemfaultdRateLimiter *handle = memfaultd_rate_limiter_init(5, 3600, NULL);
  CHECK(handle);

  time_t *history = memfaultd_rate_limiter_get_history(handle);
  for (int i = 0; i < 5; ++i) {
    CHECK_EQUAL(0, history[i]);
  }

  memfaultd_rate_limiter_destroy(handle);
}

TEST(TestGroup_Init, InitSuccessWithEmptyHistoryFile) {
  sMemfaultdRateLimiter *handle = memfaultd_rate_limiter_init(5, 3600, tmp_rate_limit_file);
  CHECK(handle);

  time_t *history = memfaultd_rate_limiter_get_history(handle);
  for (int i = 0; i < 5; ++i) {
    CHECK_EQUAL(0, history[i]);
  }

  memfaultd_rate_limiter_destroy(handle);
}

TEST(TestGroup_Init, InitSuccessWithPopulatedHistoryFile) {
  write_ratelimit_file("500 400 300 200 100 ");

  sMemfaultdRateLimiter *handle = memfaultd_rate_limiter_init(5, 3600, tmp_rate_limit_file);

  time_t *history = memfaultd_rate_limiter_get_history(handle);
  CHECK_EQUAL(500, history[0]);
  CHECK_EQUAL(400, history[1]);
  CHECK_EQUAL(300, history[2]);
  CHECK_EQUAL(200, history[3]);
  CHECK_EQUAL(100, history[4]);

  memfaultd_rate_limiter_destroy(handle);
}

TEST_GROUP_BASE(TestGroup_CheckEvent, MemfaultdRateLimiterUtest){};

TEST(TestGroup_CheckEvent, EventSuccessNoLimiting) {
  CHECK_EQUAL(true, memfaultd_rate_limiter_check_event(NULL));
}

TEST(TestGroup_CheckEvent, EventSuccessHistoryUpdated) {
  write_ratelimit_file("500 400 300 200 100 ");

  sMemfaultdRateLimiter *handle = memfaultd_rate_limiter_init(5, 3600, tmp_rate_limit_file);

  CHECK_EQUAL(true, memfaultd_rate_limiter_check_event(handle));

  time_t *history = memfaultd_rate_limiter_get_history(handle);
  CHECK(500 != history[0]);
  CHECK_EQUAL(500, history[1]);
  CHECK_EQUAL(400, history[2]);
  CHECK_EQUAL(300, history[3]);
  CHECK_EQUAL(200, history[4]);

  char expected_file[256] = {'\0'};
  snprintf(expected_file, sizeof(expected_file), "%ld 500 400 300 200 ", history[0]);

  char *actual_file = read_ratelimit_file();

  STRCMP_EQUAL(expected_file, actual_file);
  free(actual_file);

  memfaultd_rate_limiter_destroy(handle);
}

TEST(TestGroup_CheckEvent, EventLimitReached) {
  sMemfaultdRateLimiter *handle = memfaultd_rate_limiter_init(5, 3600, NULL);

  struct timeval now;
  gettimeofday(&now, NULL);

  time_t *history = memfaultd_rate_limiter_get_history(handle);
  //! Oldest record is newer than /duration/
  history[4] = now.tv_sec - 3600 + 2;

  CHECK_EQUAL(false, memfaultd_rate_limiter_check_event(handle));

  memfaultd_rate_limiter_destroy(handle);
}

TEST(TestGroup_CheckEvent, EventLimitNotReached) {
  sMemfaultdRateLimiter *handle = memfaultd_rate_limiter_init(5, 3600, NULL);

  struct timeval now;
  gettimeofday(&now, NULL);

  time_t *history = memfaultd_rate_limiter_get_history(handle);
  //! Oldest record is older than /duration/
  history[4] = now.tv_sec - 3600 - 2;

  CHECK_EQUAL(true, memfaultd_rate_limiter_check_event(handle));

  memfaultd_rate_limiter_destroy(handle);
}
