//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for reboot_last_boot_id.c
//!

#include "reboot/reboot_last_boot_id.h"

#include <CppUTest/TestHarness.h>
#include <fcntl.h>
#include <unistd.h>

#include <cstring>

#include "memfault/core/math.h"

TEST_GROUP(TestGroup_LastBootId) {
  const char *current_boot_id;
  char last_tracked_boot_id_file[PATH_MAX];

  void setup() override {
    strcpy(last_tracked_boot_id_file, "/tmp/last_tracked_boot_id.XXXXXX");
    close(mkstemp(last_tracked_boot_id_file));
    unlink(last_tracked_boot_id_file);

    current_boot_id = "12764a0c-f27b-48b3-8fe2-10fa14fa1917";
  }

  void teardown() override { unlink(last_tracked_boot_id_file); }

  void write_last_tracked_boot_id_file(const char *contents) {
    const int fd =
      open(last_tracked_boot_id_file, O_WRONLY | O_CREAT | O_EXCL | O_TRUNC, S_IRUSR | S_IWUSR);
    CHECK_TRUE(fd > 0);
    const ssize_t sz = (ssize_t)strlen(contents) + 1;
    CHECK_EQUAL(sz, write(fd, contents, sz));
    close(fd);
  }

  void check_last_tracked_boot_id(const char *expected_boot_id) {
    const int fd = open(last_tracked_boot_id_file, O_RDONLY);
    CHECK_TRUE(fd > 0);
    char tracked_boot_id[UUID_STR_LEN] = {0};
    read(fd, tracked_boot_id, sizeof(tracked_boot_id));
    close(fd);
    STRCMP_EQUAL(expected_boot_id, tracked_boot_id);
  }
};

TEST(TestGroup_LastBootId, Test_OpenFileFailed) {
  CHECK_FALSE(memfault_reboot_is_untracked_boot_id("/", current_boot_id));
}

TEST(TestGroup_LastBootId, Test_LastTrackedBootIdFileNotExistingYet) {
  CHECK_TRUE(memfault_reboot_is_untracked_boot_id(last_tracked_boot_id_file, current_boot_id));
  check_last_tracked_boot_id(current_boot_id);
}

TEST(TestGroup_LastBootId, Test_BadLastTrackedBootIdFileContents) {
  write_last_tracked_boot_id_file("NOT A UUID");
  CHECK_TRUE(memfault_reboot_is_untracked_boot_id(last_tracked_boot_id_file, current_boot_id));
  check_last_tracked_boot_id(current_boot_id);
}

TEST(TestGroup_LastBootId, Test_BootIdAlreadyTracked) {
  write_last_tracked_boot_id_file(current_boot_id);
  CHECK_FALSE(memfault_reboot_is_untracked_boot_id(last_tracked_boot_id_file, current_boot_id));
}

TEST(TestGroup_LastBootId, Test_ReadBack) {
  const char *boot_ids[] = {
    "f85c1473-2457-48da-9a13-4f903627f610",
    "951dada5-763a-4382-847f-173d8deb3fc9",
  };
  for (unsigned int i = 0; i < MEMFAULT_ARRAY_SIZE(boot_ids); ++i) {
    CHECK_TRUE(memfault_reboot_is_untracked_boot_id(last_tracked_boot_id_file, boot_ids[i]));
    check_last_tracked_boot_id(boot_ids[i]);
  }
}
