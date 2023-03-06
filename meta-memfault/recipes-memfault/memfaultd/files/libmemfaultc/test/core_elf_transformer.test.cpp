//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for core_elf_transformer.c
//!

#include "memfault-core-handler/core_elf_transformer.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>

#include <cstring>

#include "core_elf_memory_io.h"
#include "memfault/core/math.h"

extern "C" {
void memfault_core_elf_transformer_add_warning(sMemfaultCoreElfTransformer *transformer,
                                               char *warning_msg);
}

static const Elf_Ehdr s_core_elf_header_template = {
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
  .e_machine = EM_X86_64,
  .e_version = EV_CURRENT,
  .e_flags = 0,
  .e_ehsize = sizeof(Elf_Ehdr),
  .e_phentsize = sizeof(Elf_Phdr),
};

TEST_GROUP(TestGroup_Transform) {
  sMemfaultCoreElfTransformer transformer;
  sMemfaultCoreElfTransformerHandler transformer_handler = {
    .copy_proc_mem = copy_proc_mem,
  };
  const sMemfaultCoreElfMetadata metadata = {
    .linux_sdk_version = "0.4.0",
    .captured_time_epoch_s = 1663064648,  // 0x63205a48
    .device_serial = "1234ABC",
    .hardware_version = "evt",
    .software_type = "main",
    .software_version = "1.2.3",
  };

  sMemfaultCoreElfReadMemoryIO reader_io;
  sMemfaultCoreElfWriteMemoryIO writer_io;
  uint8_t *elf_input_buffer = nullptr;
  uint8_t elf_output_buffer[16 * 1024];

  void setup() override {}

  void teardown() override {
    free(elf_input_buffer);
    elf_input_buffer = nullptr;
    mock().checkExpectations();
    mock().clear();
  }

  bool transform_elf(const Elf_Ehdr *buffer, size_t buffer_size) {
    POINTERS_EQUAL(nullptr, elf_input_buffer);
    elf_input_buffer = (uint8_t *)malloc(buffer_size);
    CHECK_TRUE(elf_input_buffer != nullptr);
    memcpy(elf_input_buffer, buffer, buffer_size);
    memfault_core_elf_read_memory_io_init(&reader_io, elf_input_buffer, buffer_size);
    memfault_core_elf_write_memory_io_init(&writer_io, elf_output_buffer,
                                           sizeof(elf_output_buffer));

    memfault_core_elf_transformer_init(&transformer, &reader_io.io, &writer_io.io, &metadata,
                                       &transformer_handler);
    return memfault_core_elf_transformer_run(&transformer);
  }

  static ssize_t copy_proc_mem(sMemfaultCoreElfTransformerHandler * handler, Elf_Addr vaddr,
                               Elf64_Xword size, void *buffer) {
    ssize_t written_size = 0;
    for (size_t i = 0; i < size; i += sizeof(Elf_Addr)) {
      auto vaddr_ptr = (Elf_Addr *)&((uint8_t *)buffer)[i];
      *vaddr_ptr = ~(vaddr + i);
      written_size += sizeof(Elf_Addr);
    }
    return written_size;
  }

  size_t written_size() const { return writer_io.cursor - (uint8_t *)writer_io.buffer; }

  size_t written_num_segments() const {
    auto *output_elf_header = (Elf_Ehdr *)elf_output_buffer;
    return output_elf_header->e_phnum;
  }

  bool written_memfault_metadata_note() const {
    auto *output_elf_header = (Elf_Ehdr *)elf_output_buffer;
    auto *segments = (Elf_Phdr *)&elf_output_buffer[output_elf_header->e_phoff];
    for (size_t i = 0; i < output_elf_header->e_phnum; ++i) {
      auto *segment = &segments[i];
      if (segment->p_type == PT_NOTE) {
        const char *name = (char *)&elf_output_buffer[segment->p_offset + sizeof(Elf_Nhdr)];
        if (strcmp("Memfault", name) == 0) {
          return true;
        }
      }
    }
    return false;
  }

  Elf_Phdr written_segment_at_index(size_t i) {
    auto *output_elf_header = (Elf_Ehdr *)elf_output_buffer;
    auto *segments = (Elf_Phdr *)&elf_output_buffer[output_elf_header->e_phoff];
    return segments[i];
  }
};

/**
 * Tests that segments of types other than PT_LOAD and PT_NOTE are ignored (warning is emitted),
 * but the Memfault metadata note is added otherwise.
 */
TEST(TestGroup_Transform, Test_UnexpectedSegmentType) {
  const size_t data_size = 4;
  uint8_t buffer[sizeof(Elf_Ehdr) + sizeof(Elf_Phdr) + data_size] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr);
  elf_header->e_phnum = 1;

  auto *segment_header = (Elf_Phdr *)&buffer[sizeof(Elf_Ehdr)];
  *segment_header = (Elf_Phdr){
    .p_type = PT_DYNAMIC,
  };
  CHECK_TRUE(transform_elf(elf_header, sizeof(buffer)));

  // Only 1 NOTE segment (Memfault metadata) got emitted:
  CHECK_EQUAL(1, written_num_segments());
  CHECK_TRUE(written_memfault_metadata_note());
}

/**
 * Tests that a NOTE segment is copied to the output verbatim,
 * but the Memfault metadata note is added otherwise.
 */
TEST(TestGroup_Transform, Test_CopyNoteSegmentVerbatim) {
  const size_t data_size = 4;
  uint8_t buffer[sizeof(Elf_Ehdr) + sizeof(Elf_Phdr) + data_size] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr);
  elf_header->e_phnum = 1;

  auto *segment_header = (Elf_Phdr *)&buffer[sizeof(Elf_Ehdr)];
  *segment_header = (Elf_Phdr){
    .p_type = PT_NOTE,
    .p_offset = sizeof(Elf_Ehdr) + sizeof(Elf_Phdr),
    .p_filesz = data_size,
  };
  for (size_t i = 0; i < data_size; ++i) {
    buffer[i + segment_header->p_offset] = 'A' + i;
  }
  CHECK_TRUE(transform_elf(elf_header, sizeof(buffer)));

  // Original NOTE and Memfault metadata NOTE should get emitted:
  CHECK_EQUAL(2, written_num_segments());
  CHECK_TRUE(written_memfault_metadata_note());

  // Check original NOTE and data are added verbatim:
  const Elf_Phdr output_segment_header = written_segment_at_index(0);
  CHECK_EQUAL(segment_header->p_type, output_segment_header.p_type);
  CHECK_EQUAL(segment_header->p_filesz, output_segment_header.p_filesz);
  MEMCMP_EQUAL(&buffer[segment_header->p_offset],
               &elf_output_buffer[output_segment_header.p_offset], data_size);
}

/**
 * Tests that for a PT_LOAD segment, the copy_proc_mem callback is invoked to delegate copying the
 * segment data.
 */
TEST(TestGroup_Transform, Test_CopyLoadSegmentUsingCallback) {
  // Using more than MEMFAULT_CORE_ELF_TRANSFORMER_PROC_MEM_COPY_BUFFER_SIZE_BYTES to exercise
  // copying in multiple chunks:
  const size_t data_size = 2 * MEMFAULT_CORE_ELF_TRANSFORMER_PROC_MEM_COPY_BUFFER_SIZE_BYTES;
  const Elf_Addr vaddr = 0x5;
  uint8_t buffer[sizeof(Elf_Ehdr) + sizeof(Elf_Phdr) + data_size] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr);
  elf_header->e_phnum = 1;

  auto *segment_header = (Elf_Phdr *)&buffer[sizeof(Elf_Ehdr)];
  *segment_header = (Elf_Phdr){
    .p_type = PT_LOAD,
    .p_offset = sizeof(Elf_Ehdr) + sizeof(Elf_Phdr),
    .p_vaddr = vaddr,
    .p_filesz = data_size,
  };
  CHECK_TRUE(transform_elf(elf_header, sizeof(buffer)));

  // Original NOTE and Memfault metadata NOTE should get emitted:
  CHECK_EQUAL(2, written_num_segments());
  CHECK_TRUE(written_memfault_metadata_note());

  // Check original NOTE and data are added verbatim:
  const Elf_Phdr output_segment_header = written_segment_at_index(0);
  CHECK_EQUAL(segment_header->p_type, output_segment_header.p_type);
  CHECK_EQUAL(segment_header->p_filesz, output_segment_header.p_filesz);
  CHECK_EQUAL(segment_header->p_vaddr, output_segment_header.p_vaddr);

  // The copy_proc_mem implementation writes the inverted vaddr's as segment data:
  for (size_t i = 0; i < output_segment_header.p_filesz; i += sizeof(Elf_Addr)) {
    auto output_vaddr = (Elf_Addr *)&elf_output_buffer[output_segment_header.p_offset + i];
    CHECK_EQUAL(~(vaddr + i), *output_vaddr);
  }
}

/**
 * Tests that reaching the maximum number of warnings does not result in a crash.
 */
TEST(TestGroup_Transform, Test_ExceedMaxWarnings) {
  const size_t data_size = 4;
  uint8_t buffer[sizeof(Elf_Ehdr) + sizeof(Elf_Phdr) + data_size] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr);
  elf_header->e_phnum = 1;

  auto *segment_header = (Elf_Phdr *)&buffer[sizeof(Elf_Ehdr)];
  *segment_header = (Elf_Phdr){
    .p_type = PT_DYNAMIC,
  };
  memfault_core_elf_read_memory_io_init(&reader_io, buffer, sizeof(buffer));
  memfault_core_elf_write_memory_io_init(&writer_io, elf_output_buffer, sizeof(elf_output_buffer));

  memfault_core_elf_transformer_init(&transformer, &reader_io.io, &writer_io.io, &metadata,
                                     &transformer_handler);

  for (size_t i = 0; i < MEMFAULT_ARRAY_SIZE(transformer.warnings) + 1; ++i) {
    memfault_core_elf_transformer_add_warning(&transformer, strdup("warning"));
  }

  CHECK_TRUE(memfault_core_elf_transformer_run(&transformer));

  // Only 1 NOTE segment (Memfault metadata) got emitted:
  CHECK_EQUAL(1, written_num_segments());
  CHECK_TRUE(written_memfault_metadata_note());
}
