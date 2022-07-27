//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for device_settings.c
//!

#include "device_settings.h"

#include <CppUTest/TestHarness.h>

#include <cstring>

TEST_GROUP(TestDeviceSettingsGroup){};

static const char** current_fgetsArray = NULL;
static int current_fgetsArray_len = 0;
static FILE* current_popenResponse = NULL;

FILE* popen(const char* command, const char* type) { return current_popenResponse; }

int pclose(FILE* stream) { return 0; }

char* fgets(char* s, int size, FILE* stream) {
  static int i = 0;

  if (i >= current_fgetsArray_len) {
    i = 0;
    return NULL;
  }

  strcpy(s, current_fgetsArray[i]);
  ++i;
  return s;
}

/**
 * @brief Success test case, memfault-device-info is successfully callable and returns all expected
 * values
 *
 */
TEST(TestDeviceSettingsGroup, SuccessTest) {
  current_popenResponse = (FILE*)1;

  static const char* successTest_fgetsArray[] = {"MEMFAULT_DEVICE_ID=device_id",
                                                 "MEMFAULT_HARDWARE_VERSION=hardware_version"};

  current_fgetsArray = successTest_fgetsArray;
  current_fgetsArray_len = sizeof(successTest_fgetsArray) / sizeof(char*);

  sMemfaultdDeviceSettings* settings = memfaultd_device_settings_init();
  CHECK(settings != NULL);

  STRCMP_EQUAL(settings->device_id, "device_id");
  STRCMP_EQUAL(settings->hardware_version, "hardware_version");

  memfaultd_device_settings_destroy(settings);
}

/**
 * @brief Failure test case, memfault-device-info is successfully callable but does not return all
 * expected values
 *
 */
TEST(TestDeviceSettingsGroup, MissingEntryTest) {
  current_popenResponse = (FILE*)1;

  static const char* missingEntryTest_fgetsArray[] = {"MEMFAULT_DEVICE_ID = device_id"};
  current_fgetsArray = missingEntryTest_fgetsArray;
  current_fgetsArray_len = sizeof(missingEntryTest_fgetsArray) / sizeof(char*);

  sMemfaultdDeviceSettings* settings = memfaultd_device_settings_init();
  POINTERS_EQUAL(NULL, settings);
}

/**
 * @brief Failure test case, memfault-device-info is not callable
 *
 */
TEST(TestDeviceSettingsGroup, popenFailureTest) {
  current_popenResponse = (FILE*)NULL;

  sMemfaultdDeviceSettings* settings = memfaultd_device_settings_init();
  POINTERS_EQUAL(NULL, settings);
}
