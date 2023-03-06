//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for core_elf_metadata.c
//!

#include "memfault-core-handler/core_elf_metadata.h"

#include <CppUTest/TestHarness.h>

#include <cstring>

#include "hex2bin.h"

TEST_GROUP(TestGroup_CoreElfMetadata){};

TEST(TestGroup_CoreElfMetadata, Test_WriteMetadata) {
  const sMemfaultCoreElfMetadata metadata = {
    .linux_sdk_version = "0.4.0",
    .captured_time_epoch_s = 1663064648,
    .device_serial = "1234ABC",
    .hardware_version = "evt",
    .software_type = "main",
    .software_version = "1.2.3",
  };
  size_t note_buffer_size = memfault_core_elf_metadata_note_calculate_size(&metadata);
  uint8_t note_buffer[note_buffer_size];
  memset(note_buffer, 0xAA, note_buffer_size);

  CHECK_TRUE(memfault_core_elf_metadata_note_write(&metadata, note_buffer, note_buffer_size));

  size_t expected_buffer_size;
  uint8_t *const expected_buffer_contents = memfault_hex2bin(
    // namesz
    "09000000"
    // descsz
    "2B000000"
    // name ("Memfault")
    "4D4554414D656D6661756C7400"
    // name padding
    "000000"
    // desc (CBOR data)
    "A7"              // map(7)
    "01"              // Schema Version
    "01"              // unsigned(1)
    "02"              // Linux SDK Version
    "65"              // text(5)
    "302E342E30"      // "0.4.0"
    "03"              // Captured Time
    "1A63205A48"      // unsigned(1663064648)
    "04"              // Device Serial
    "67"              // text(7)
    "31323334414243"  // "1234ABC"
    "05"              // Hardware Version
    "63"              // text(3)
    "657674"          // "evt"
    "06"              // Software Type
    "64"              // text(4)
    "6D61696E"        // "main"
    "07"              // Software Version
    "65"              // text(5)
    "312E322E33"      //"1.2.3"
    // desc padding
    "00",
    &expected_buffer_size);
  MEMCMP_EQUAL(expected_buffer_contents, note_buffer, expected_buffer_size);

  free(expected_buffer_contents);
}
