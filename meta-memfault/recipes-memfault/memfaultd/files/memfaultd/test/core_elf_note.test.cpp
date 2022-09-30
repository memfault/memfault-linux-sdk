//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for core_elf_note.c
//!

#include "coredump/core_elf_note.h"

#include <CppUTest/TestHarness.h>

#include <cstring>

#include "hex2bin.h"

TEST_GROUP(TestGroup_ElfNotes){};

static const Elf_Word s_test_note_type = 0x12345678;

TEST(TestGroup_ElfNotes, Test_NoteWriting) {
  struct {
    const char *owner_name;
    size_t description_size;
    const char *expected_buffer_contents_hex;
  } s_cases[] = {
    // Header-only size in case there are no name and description:
    {
      .owner_name = "",
      .description_size = 0,
      .expected_buffer_contents_hex = "00000000"
                                      "00000000"
                                      "78563412",
    },
    // Description data is padded to 4-byte alignment:
    {
      .owner_name = "",
      .description_size = 1,
      .expected_buffer_contents_hex = "00000000"
                                      "01000000"
                                      "78563412"
                                      "FF000000",
    },
    // Description data already 4-byte aligned:
    {
      .owner_name = "",
      .description_size = 4,
      .expected_buffer_contents_hex = "00000000"
                                      "04000000"
                                      "78563412"
                                      "FFFFFFFF",
    },
    // Name data and size includes NUL terminator and is padded to 4-byte alignment:
    {
      .owner_name = "ABC",
      .description_size = 0,
      .expected_buffer_contents_hex = "04000000"
                                      "00000000"
                                      "78563412"
                                      "41424300",
    },
    // Both name and description:
    {
      .owner_name = "A",
      .description_size = 1,
      .expected_buffer_contents_hex = "02000000"
                                      "01000000"
                                      "78563412"
                                      "41000000"
                                      "FF000000",
    },
  };

  for (auto &s_case : s_cases) {
    size_t expected_buffer_size;
    uint8_t *const expected_buffer_contents =
      memfault_hex2bin(s_case.expected_buffer_contents_hex, &expected_buffer_size);

    const size_t buffer_size =
      memfault_core_elf_note_calculate_size(s_case.owner_name, s_case.description_size);
    CHECK_EQUAL(expected_buffer_size, buffer_size);

    uint8_t buffer[buffer_size];
    // Write canary values for debugging (0xAA bytes):
    memset(buffer, 0xAA, buffer_size);

    uint8_t *const desc_buffer = memfault_core_elf_note_init(
      buffer, s_case.owner_name, s_case.description_size, s_test_note_type);
    // Write the description data (0xFF bytes):
    memset(desc_buffer, 0xFF, s_case.description_size);

    MEMCMP_EQUAL(expected_buffer_contents, buffer, expected_buffer_size);
    free(expected_buffer_contents);
  }
}
