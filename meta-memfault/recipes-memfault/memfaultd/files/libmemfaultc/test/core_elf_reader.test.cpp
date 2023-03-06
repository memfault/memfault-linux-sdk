//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for core_elf_reader.c
//!

#include "memfault-core-handler/core_elf_reader.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>

#include <cstring>

#include "core_elf_memory_io.h"

TEST_GROUP(TestGroup_ElfIsValid){};

TEST(TestGroup_ElfIsValid, Test_NullBuffer) {
  CHECK_FALSE(memfault_core_elf_reader_is_valid_core_elf(NULL, 1024));
}

TEST(TestGroup_ElfIsValid, Test_InputTooSmall) {
  uint8_t buffer[] = "\177ELF";
  CHECK_FALSE(memfault_core_elf_reader_is_valid_core_elf(buffer, sizeof(buffer)));
}

TEST(TestGroup_ElfIsValid, Test_InvalidMagic) {
  const Elf_Ehdr elf_header = (Elf_Ehdr){
    .e_ident =
      {
        '\x7f',
        'E',
        'L',
        'V',
        ELFCLASS,
      },
    .e_type = ET_CORE,
    .e_version = EV_CURRENT,
    .e_ehsize = sizeof(Elf_Ehdr),
    .e_phentsize = sizeof(Elf_Phdr),
  };
  CHECK_FALSE(memfault_core_elf_reader_is_valid_core_elf(&elf_header, sizeof(elf_header)));
}

TEST(TestGroup_ElfIsValid, Test_NotACoreElf) {
  const Elf_Ehdr elf_header = (Elf_Ehdr){
    .e_ident =
      {
        '\x7f',
        'E',
        'L',
        'F',
        ELFCLASS,
      },
    .e_type = ET_EXEC,
    .e_version = EV_CURRENT,
    .e_ehsize = sizeof(Elf_Ehdr),
    .e_phentsize = sizeof(Elf_Phdr),
  };
  CHECK_FALSE(memfault_core_elf_reader_is_valid_core_elf(&elf_header, sizeof(elf_header)));
}

TEST(TestGroup_ElfIsValid, Test_UnsupportedPhentsize) {
  const Elf_Ehdr elf_header = (Elf_Ehdr){
    .e_ident =
      {
        '\x7f',
        'E',
        'L',
        'F',
        ELFCLASS,
      },
    .e_type = ET_CORE,
    .e_version = EV_CURRENT,
    .e_ehsize = sizeof(Elf_Ehdr),
    .e_phentsize = sizeof(Elf_Phdr) - 4,
  };
  CHECK_FALSE(memfault_core_elf_reader_is_valid_core_elf(&elf_header, sizeof(elf_header)));
}

TEST(TestGroup_ElfIsValid, Test_UnsupportedEhsize) {
  const Elf_Ehdr elf_header = (Elf_Ehdr){
    .e_ident =
      {
        '\x7f',
        'E',
        'L',
        'F',
        ELFCLASS,
      },
    .e_type = ET_CORE,
    .e_version = EV_CURRENT,
    .e_ehsize = sizeof(Elf_Ehdr) - 4,
    .e_phentsize = sizeof(Elf_Phdr),
  };
  CHECK_FALSE(memfault_core_elf_reader_is_valid_core_elf(&elf_header, sizeof(elf_header)));
}

TEST(TestGroup_ElfIsValid, Test_Valid) {
  const Elf_Ehdr elf_header = (Elf_Ehdr){
    .e_ident =
      {
        '\x7f',
        'E',
        'L',
        'F',
        ELFCLASS,
      },
    .e_type = ET_CORE,
    .e_version = EV_CURRENT,
    .e_ehsize = sizeof(Elf_Ehdr),
    .e_phentsize = sizeof(Elf_Phdr),
  };
  CHECK_TRUE(memfault_core_elf_reader_is_valid_core_elf(&elf_header, sizeof(elf_header)));
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

static void (*s_handle_segments_hook)(sMemfaultCoreElfReader *reader) = nullptr;

TEST_GROUP(TestGroup_ElfReaderReadAll) {
  sMemfaultCoreElfReader reader;
  sMemfaultCoreElfReadMemoryIO mio;
  sMemfaultCoreElfReaderHandler handler = {
    .handle_elf_header = handle_elf_header,
    .handle_segments = handle_segments,
    .handle_warning = handle_warning,
    .handle_done = handle_done,
  };

  uint8_t *elf_buffer = nullptr;

  static void handle_elf_header(sMemfaultCoreElfReader * reader, const Elf_Ehdr *elf_header) {
    mock()
      .actualCall("handle_elf_header")
      .withPointerParameter("reader", reader)
      .withMemoryBufferParameter("elf_header", (unsigned char *)elf_header, sizeof(*elf_header));
  }

  static void handle_segments(sMemfaultCoreElfReader * reader, const Elf_Phdr *segments,
                              size_t num_segments) {
    mock()
      .actualCall("handle_segments")
      .withPointerParameter("reader", reader)
      .withMemoryBufferParameter("segments", (unsigned char *)segments,
                                 sizeof(*segments) * num_segments)
      .withUnsignedIntParameter("num_segments", num_segments);

    if (s_handle_segments_hook != nullptr) {
      s_handle_segments_hook(reader);
    }
  }

  static void handle_warning(sMemfaultCoreElfReader * reader, char *msg) {
    mock()
      .actualCall("handle_warning")
      .withPointerParameter("reader", reader)
      .withStringParameter("msg", msg);
    free(msg);
  }

  static void handle_done(sMemfaultCoreElfReader * reader) {
    mock().actualCall("handle_done").withPointerParameter("reader", reader);
  }

  void teardown() override {
    free(elf_buffer);
    elf_buffer = nullptr;
    s_handle_segments_hook = nullptr;
    mock().checkExpectations();
    mock().clear();
  }

  void setup_elf(const Elf_Ehdr *buffer, size_t buffer_size) {
    POINTERS_EQUAL(nullptr, elf_buffer);
    elf_buffer = (uint8_t *)malloc(buffer_size);
    CHECK_TRUE(elf_buffer != nullptr);
    memcpy(elf_buffer, buffer, buffer_size);
    memfault_core_elf_read_memory_io_init(&mio, elf_buffer, buffer_size);
    memfault_core_elf_reader_init(&reader, &mio.io, &handler);
  }

  void test_read_segment_data_elf(void (*hook)(sMemfaultCoreElfReader *)) {
    const size_t data_size = 4;
    uint8_t buffer[sizeof(Elf_Ehdr) + data_size] = {0};
    auto *elf_header = (Elf_Ehdr *)buffer;
    *elf_header = s_core_elf_header_template;
    elf_header->e_phoff = sizeof(Elf_Ehdr);
    elf_header->e_phnum = 0;
    // Fill data with ABC...:
    for (size_t i = 0; i < data_size; ++i) {
      buffer[i + sizeof(Elf_Ehdr)] = 'A' + i;
    }

    setup_elf((Elf_Ehdr *)buffer, sizeof(buffer));

    mock()
      .expectOneCall("handle_segments")
      .withPointerParameter("reader", &reader)
      .withMemoryBufferParameter("segments", nullptr, 0)
      .withUnsignedIntParameter("num_segments", 0);
    mock().ignoreOtherCalls();

    s_handle_segments_hook = hook;
    memfault_core_elf_reader_read_all(&reader);
  }
};

/**
 * Tests that handle_warning gets called when the stream ends unexpectedly.
 */
TEST(TestGroup_ElfReaderReadAll, Test_WarningForUnexpectedEOF) {
  // Note: only using one byte of the header!
  setup_elf(&s_core_elf_header_template, 1);

  mock()
    .expectOneCall("handle_warning")
    .withPointerParameter("reader", &reader)
    .withStringParameter("msg", "Unexpected short read while reading ELF header");

  mock().expectOneCall("handle_done").withPointerParameter("reader", &reader);

  memfault_core_elf_reader_read_all(&reader);
};

/**
 * Tests that handle_warning gets called when an header is parsed that is not supported.
 */
TEST(TestGroup_ElfReaderReadAll, Test_WarningForInvalidHeader) {
  Elf_Ehdr elf_header = s_core_elf_header_template;
  elf_header.e_type = ET_EXEC;
  setup_elf(&elf_header, sizeof(elf_header));

  mock()
    .expectOneCall("handle_warning")
    .withPointerParameter("reader", &reader)
    .withStringParameter("msg", "Not an ELF coredump");

  mock().expectOneCall("handle_done").withPointerParameter("reader", &reader);

  memfault_core_elf_reader_read_all(&reader);
};

/**
 * Test that handle_elf_header gets called once when a valid ELF (header only) is parsed.
 */
TEST(TestGroup_ElfReaderReadAll, Test_ElfHeaderOk) {
  const Elf_Ehdr elf_header = s_core_elf_header_template;
  setup_elf(&elf_header, sizeof(elf_header));

  mock()
    .expectOneCall("handle_elf_header")
    .withPointerParameter("reader", &reader)
    .withMemoryBufferParameter("elf_header", (unsigned char *)&elf_header, sizeof(elf_header));

  // Note: e_phoff is zero in elf_header, hence the warning:
  mock()
    .expectOneCall("handle_warning")
    .withPointerParameter("reader", &reader)
    .withStringParameter("msg", "Unexpected segment table offset");

  mock().expectOneCall("handle_done").withPointerParameter("reader", &reader);

  memfault_core_elf_reader_read_all(&reader);
};

/**
 * The Linux kernel writes the segment table immediately after the ELF header.
 * Ensure that we'll get a warning in case this ever changes.
 */
TEST(TestGroup_ElfReaderReadAll, Test_WarnIfDataBetweenHeaderAndSegmentTable) {
  // Note: 8 bytes of data between the ELF header and segment table:
  const size_t gap_size = 8;
  uint8_t buffer[sizeof(Elf_Ehdr) + gap_size + sizeof(Elf_Phdr)] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr) + gap_size;
  elf_header->e_phnum = 1;

  auto *segment_header = (Elf_Phdr *)(buffer + sizeof(Elf_Ehdr) + gap_size);
  *segment_header = (Elf_Phdr){
    .p_offset = sizeof(buffer),
    .p_filesz = 0,
  };

  setup_elf((Elf_Ehdr *)buffer, sizeof(buffer));

  mock()
    .expectOneCall("handle_elf_header")
    .withPointerParameter("reader", &reader)
    .withMemoryBufferParameter("elf_header", (unsigned char *)elf_header, sizeof(*elf_header));

  mock()
    .expectOneCall("handle_warning")
    .withPointerParameter("reader", &reader)
    .withStringParameter("msg", "Ignoring data between header and segment table");

  mock()
    .expectOneCall("handle_segments")
    .withPointerParameter("reader", &reader)
    .withMemoryBufferParameter("segments", (unsigned char *)segment_header, sizeof(*segment_header))
    .withUnsignedIntParameter("num_segments", 1);

  mock().expectOneCall("handle_done").withPointerParameter("reader", &reader);

  memfault_core_elf_reader_read_all(&reader);
};

/**
 * Test that the reader gracefully fails if the segment table is incomplete.
 */
TEST(TestGroup_ElfReaderReadAll, Test_IncompleteSegmentsTable) {
  // Note: segment table is 1 byte short
  uint8_t buffer[sizeof(Elf_Ehdr) + sizeof(Elf_Phdr) - 1] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr);
  elf_header->e_phnum = 1;

  const Elf_Phdr segment_header = (Elf_Phdr){
    .p_offset = sizeof(buffer),
    .p_filesz = 0,
  };
  memcpy(buffer + sizeof(Elf_Ehdr), &segment_header, sizeof(segment_header) - 1);

  setup_elf((Elf_Ehdr *)buffer, sizeof(buffer));

  mock()
    .expectOneCall("handle_elf_header")
    .withPointerParameter("reader", &reader)
    .withMemoryBufferParameter("elf_header", (unsigned char *)elf_header, sizeof(*elf_header));

  mock()
    .expectOneCall("handle_warning")
    .withPointerParameter("reader", &reader)
    .withStringParameter("msg", "Unexpected short read while reading segment headers");

  mock().expectOneCall("handle_done").withPointerParameter("reader", &reader);

  memfault_core_elf_reader_read_all(&reader);
};

/**
 * Test that the reader calls handle_segments when it has read all the segment headers.
 */
TEST(TestGroup_ElfReaderReadAll, Test_HandleSegments) {
  uint8_t buffer[sizeof(Elf_Ehdr) + 2 * sizeof(Elf_Phdr)] = {0};
  auto *elf_header = (Elf_Ehdr *)buffer;
  *elf_header = s_core_elf_header_template;
  elf_header->e_phoff = sizeof(Elf_Ehdr);
  elf_header->e_phnum = 2;

  auto *segment_header = (Elf_Phdr *)&buffer[sizeof(Elf_Ehdr)];
  segment_header[0] = (Elf_Phdr){
    .p_offset = sizeof(buffer),
    .p_filesz = 1234,
  };
  segment_header[1] = (Elf_Phdr){
    .p_offset = sizeof(buffer),
    .p_filesz = 5678,
  };

  setup_elf((Elf_Ehdr *)buffer, sizeof(buffer));

  mock()
    .expectOneCall("handle_elf_header")
    .withPointerParameter("reader", &reader)
    .withMemoryBufferParameter("elf_header", (unsigned char *)elf_header, sizeof(*elf_header));

  mock()
    .expectOneCall("handle_segments")
    .withPointerParameter("reader", &reader)
    .withMemoryBufferParameter("segments", (unsigned char *)segment_header,
                               sizeof(*segment_header) * 2)
    .withUnsignedIntParameter("num_segments", 2);

  mock().expectOneCall("handle_done").withPointerParameter("reader", &reader);

  memfault_core_elf_reader_read_all(&reader);
};

static void prv_past_stream_position_hook(sMemfaultCoreElfReader *reader) {
  uint8_t buffer[] = {0};
  const size_t expected_read_size = 0;
  CHECK_EQUAL(expected_read_size,
              memfault_core_elf_reader_read_segment_data(reader, 0, buffer, sizeof(buffer)));
}

/**
 * Tests that calling memfault_core_elf_reader_read_segment_data() with an at_pos that has already
 * been read from the stream, will return 0.
 */
TEST(TestGroup_ElfReaderReadAll, Test_ReadSegmentDataPastStreamPosition) {
  test_read_segment_data_elf(prv_past_stream_position_hook);
}

static void prv_skip_to_position_hook(sMemfaultCoreElfReader *reader) {
  uint8_t buffer[2] = {0};
  // Skip over "AB":
  const size_t pos = sizeof(Elf_Ehdr) + 2;
  const size_t expected_read_size = 2;
  CHECK_EQUAL(expected_read_size,
              memfault_core_elf_reader_read_segment_data(reader, pos, buffer, sizeof(buffer)));
  MEMCMP_EQUAL("CD", buffer, sizeof(buffer));
}

/**
 * Tests that calling memfault_core_elf_reader_read_segment_data() with an at_pos that is past the
 * current stream position, will skip to the requested position and read from there.
 */
TEST(TestGroup_ElfReaderReadAll, Test_ReadSegmentSkipToPosition) {
  test_read_segment_data_elf(prv_skip_to_position_hook);
}

static void prv_skip_to_eof_hook(sMemfaultCoreElfReader *reader) {
  uint8_t buffer[1] = {0};
  // Requested pos is past EOF:
  const size_t pos = sizeof(Elf_Ehdr) + 5;
  const size_t expected_read_size = 0;
  CHECK_EQUAL(expected_read_size,
              memfault_core_elf_reader_read_segment_data(reader, pos, buffer, sizeof(buffer)));
}

/**
 * Tests that calling memfault_core_elf_reader_read_segment_data() with an at_pos that is past the
 * end of the stream, will return 0.
 */
TEST(TestGroup_ElfReaderReadAll, Test_ReadSegmentSkipToEOF) {
  test_read_segment_data_elf(prv_skip_to_eof_hook);
}

static void prv_read_until_hook(sMemfaultCoreElfReader *reader) {
  uint8_t buffer[10] = {0};
  const size_t pos = sizeof(Elf_Ehdr);
  // Only 4 bytes are copied, even though we asked for 10.
  const size_t expected_read_size = 4;
  CHECK_EQUAL(expected_read_size,
              memfault_core_elf_reader_read_segment_data(reader, pos, buffer, sizeof(buffer)));
  MEMCMP_EQUAL("ABCD", buffer, sizeof(buffer));
}

/**
 * Tests that calling memfault_core_elf_reader_read_segment_data() with a length that is past the
 * end of the stream, will copy the bytes up until the end of the stream.
 */
TEST(TestGroup_ElfReaderReadAll, Test_ReadSegmentReadUntilEOF) {
  test_read_segment_data_elf(prv_read_until_hook);
}
