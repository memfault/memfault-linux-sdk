//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for core_elf_writer.c
//!

#include "coredump/core_elf_writer.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>

#include "core_elf_memory_io.h"
#include "memfault/core/math.h"

TEST_GROUP(TestGroup_ElfWriting) {
  uint8_t buffer[1024];
  sMemfaultCoreElfWriteMemoryIO mio;
  sMemfaultCoreElfWriter writer;
  Elf_Half elf_machine;
  Elf_Word elf_flags;

  void setup() override {
    memfault_core_elf_write_memory_io_init(&mio, buffer, sizeof(buffer));
    memfault_core_elf_writer_init(&writer, &mio.io);

    elf_machine = EM_X86_64;
    elf_flags = 0x7;
    memfault_core_elf_writer_set_elf_header_fields(&writer, elf_machine, elf_flags);
  }

  void teardown() override {
    memfault_core_elf_writer_finalize(&writer);
    mock().checkExpectations();
    mock().clear();
  }

  size_t written_size() const { return mio.cursor - (uint8_t *)mio.buffer; }
};

TEST(TestGroup_ElfWriting, Test_WriteEmptyElf) {
  Elf_Ehdr elf_header = {
    .e_ident =
      {
        ELFMAG0,
        ELFMAG1,
        ELFMAG2,
        ELFMAG3,
        ELFCLASS,
        ELFDATA,
        EV_CURRENT,
      },
    .e_type = ET_CORE,
    .e_machine = elf_machine,
    .e_version = EV_CURRENT,
    .e_flags = elf_flags,
    .e_ehsize = sizeof(Elf_Ehdr),
    // Note: e_phentsize and e_phnum are zero
  };
  CHECK_TRUE(memfault_core_elf_writer_write(&writer));
  CHECK_EQUAL(sizeof(elf_header), written_size());
  MEMCMP_EQUAL(&elf_header, mio.buffer, sizeof(elf_header));
}

TEST(TestGroup_ElfWriting, Test_WriteSegmentDataWithBuffer) {
  const size_t data_size = 4;
  auto *data = (uint8_t *)malloc(data_size);
  for (size_t i = 0; i < data_size; ++i) {
    data[i] = 'A' + i;
  }
  Elf_Phdr segment = {
    .p_type = PT_LOAD,
    .p_vaddr = 0x12345678,
    .p_filesz = data_size,
  };
  segment.p_flags = 0xFF;  // work-around for struct fields ordered differently for 64 vs 32-bit
  CHECK_TRUE(memfault_core_elf_writer_add_segment_with_buffer(&writer, &segment, data));
  CHECK_TRUE(memfault_core_elf_writer_write(&writer));

  // Expect to be placed immediately after the ELF header and segment table:
  Elf_Phdr expected_segment = segment;
  expected_segment.p_offset = sizeof(Elf_Ehdr) + sizeof(Elf_Phdr);
  MEMCMP_EQUAL(&expected_segment, &buffer[sizeof(Elf_Ehdr)], sizeof(Elf_Phdr));

  // Segment data is placed immediately after the segment table:
  MEMCMP_EQUAL(data, &buffer[expected_segment.p_offset], data_size);
}

static bool prv_segment_data_callback(void *ctx, const Elf_Phdr *segment) {
  CHECK_EQUAL(PT_LOAD, segment->p_type);
  CHECK_EQUAL(0xFF, segment->p_flags);
  CHECK_EQUAL(0x12345678, segment->p_vaddr);
  const size_t data_size = 4;
  memfault_core_elf_writer_write_segment_data((sMemfaultCoreElfWriter *)ctx, "ABCD", data_size);
  return true;
}

TEST(TestGroup_ElfWriting, Test_WriteSegmentDataWithCallback) {
  const size_t data_size = 4;
  Elf_Phdr segment = {
    .p_type = PT_LOAD,
    .p_vaddr = 0x12345678,
    .p_filesz = data_size,
  };
  segment.p_flags = 0xFF;  // work-around for struct fields ordered differently for 64 vs 32-bit
  CHECK_TRUE(memfault_core_elf_writer_add_segment_with_callback(
    &writer, &segment, prv_segment_data_callback, &writer));
  CHECK_TRUE(memfault_core_elf_writer_write(&writer));

  // Expect to be placed immediately after the ELF header and segment table:
  Elf_Phdr expected_segment = segment;
  expected_segment.p_offset = sizeof(Elf_Ehdr) + sizeof(Elf_Phdr);
  MEMCMP_EQUAL(&expected_segment, &buffer[sizeof(Elf_Ehdr)], sizeof(Elf_Phdr));

  // Segment data is placed immediately after the segment table:
  MEMCMP_EQUAL("ABCD", &buffer[expected_segment.p_offset], data_size);
}

TEST(TestGroup_ElfWriting, Test_WriteSegmentDataRequiringPadding) {
  const size_t data_size = 4;
  auto *data = (uint8_t *)malloc(data_size);
  for (size_t i = 0; i < data_size; ++i) {
    data[i] = 'A' + i;
  }
  const size_t alignment = 1 << 7;
  Elf_Phdr segment = {
    .p_type = PT_LOAD,
    .p_vaddr = 0x12345678,
    .p_filesz = data_size,
    .p_align = alignment,
  };
  segment.p_flags = 0xFF;  // work-around for struct fields ordered differently for 64 vs 32-bit
  CHECK_TRUE(memfault_core_elf_writer_add_segment_with_buffer(&writer, &segment, data));
  CHECK_TRUE(memfault_core_elf_writer_write(&writer));

  // Expect to be placed with padding after the ELF header and segment table:
  Elf_Phdr expected_segment = segment;
  expected_segment.p_offset = MEMFAULT_ALIGN_UP(sizeof(Elf_Ehdr) + sizeof(Elf_Phdr), alignment);
  MEMCMP_EQUAL(&expected_segment, &buffer[sizeof(Elf_Ehdr)], sizeof(Elf_Phdr));

  // Segment data is placed after the segment table and the padding:
  MEMCMP_EQUAL(data, &buffer[expected_segment.p_offset], data_size);
}
