//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for reboot_process_pstore.c
//!

#include "reboot/reboot_process_pstore.h"

#include <CppUTest/TestHarness.h>
#include <sys/stat.h>
#include <unistd.h>

#include <cstring>

TEST_GROUP(TestGroup_ProcessPstore) {
  char pstore_dir[PATH_MAX];

  void setup() override {
    strcpy(pstore_dir, "/tmp/pstore.XXXXXX");
    mkdtemp(pstore_dir);
  }

  void teardown() override { rmdir(pstore_dir); }

  void createFile(std::string & path) {
    fprintf(stderr, ">>>> %s\n", path.c_str());
    FILE *file = fopen(path.c_str(), "w+");
    fclose(file);
  }

  void check_file_does_not_exist(std::string & path) {
    CHECK_EQUAL(-1, access(path.c_str(), F_OK));
    CHECK_EQUAL(ENOENT, errno);
  }
};

TEST(TestGroup_ProcessPstore, Test_ClearsDotFile) {
  auto file = std::string(pstore_dir) + "/.dotfile";
  createFile(file);
  memfault_reboot_process_pstore_files(pstore_dir);
  check_file_does_not_exist(file);
}

TEST(TestGroup_ProcessPstore, Test_ClearsRegularFile) {
  auto file = std::string(pstore_dir) + "/regular_file";
  createFile(file);
  memfault_reboot_process_pstore_files(pstore_dir);
  check_file_does_not_exist(file);
}

TEST(TestGroup_ProcessPstore, Test_ClearsBrokenSymlink) {
  auto file = std::string(pstore_dir) + "/symlink";
  std::string target = "/nowhere";
  check_file_does_not_exist(target);
  symlink(file.c_str(), target.c_str());
  memfault_reboot_process_pstore_files(pstore_dir);
  check_file_does_not_exist(file);
}

TEST(TestGroup_ProcessPstore, Test_RemovesSymlinkButKeepsTarget) {
  char target_path[PATH_MAX] = "/tmp/target.XXXXXX";
  int fd = mkstemp(target_path);
  CHECK(fd > 0);

  auto file = std::string(pstore_dir) + "/ext_symlink";
  std::string target = target_path;
  symlink(file.c_str(), target.c_str());
  memfault_reboot_process_pstore_files(pstore_dir);
  check_file_does_not_exist(file);

  // Target file still exists:
  CHECK_EQUAL(0, access(target_path, F_OK));

  // Clean up:
  close(fd);
  unlink(target_path);
}

TEST(TestGroup_ProcessPstore, Test_ClearsFileInDirectory) {
  auto subdir = std::string(pstore_dir) + "/subdir";
  auto file = subdir + "/regular_file";
  mkdir(subdir.c_str(), 0755);
  createFile(file);
  memfault_reboot_process_pstore_files(pstore_dir);
  check_file_does_not_exist(file);
  // Note: the directories themselves are not removed
  rmdir(subdir.c_str());
}
