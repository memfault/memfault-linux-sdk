//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for queue.c
//!

#include "queue.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>
#include <fcntl.h>
#include <unistd.h>

#include <cstring>

#include "hex2bin.h"

extern "C" {
bool memfaultd_queue_is_file_backed(sMemfaultdQueue *handle);
int memfaultd_queue_get_size(sMemfaultdQueue *handle);
uint32_t memfaultd_queue_get_read_ptr(sMemfaultdQueue *handle);
uint32_t memfaultd_queue_get_write_ptr(sMemfaultdQueue *handle);
uint32_t memfaultd_queue_get_prev_ptr(sMemfaultdQueue *handle);
}

static sMemfaultd *g_stub_memfaultd = (sMemfaultd *)~0;

char *memfaultd_generate_rw_filename(sMemfaultd *memfaultd, const char *filename) {
  const char *path = mock()
                       .actualCall("memfaultd_generate_rw_filename")
                       .withPointerParameter("memfaultd", memfaultd)
                       .withStringParameter("filename", filename)
                       .returnStringValue();
  return strdup(path);  // original returns malloc'd string
}

TEST_BASE(MemfaultdQueueUtest) {
  char tmp_dir[PATH_MAX] = {0};
  char tmp_queue_file[4200] = {0};

  void setup() override {
    strcpy(tmp_dir, "/tmp/memfaultd.XXXXXX");
    mkdtemp(tmp_dir);
    sprintf(tmp_queue_file, "%s/queue", tmp_dir);
  }

  void teardown() override {
    unlink(tmp_dir);
    mock().clear();
  }

  void expect_queue_file_get_string_call(const char *path) {
    mock()
      .expectOneCall("memfaultd_generate_rw_filename")
      .withPointerParameter("memfaultd", g_stub_memfaultd)
      .withStringParameter("filename", "queue")
      .andReturnValue(path);
  }

  void create_queue_file(const char *hex_contents) {
    memfault_hex2bin_file(tmp_queue_file, hex_contents);
    expect_queue_file_get_string_call(tmp_queue_file);
  }

  void check_queue_file_contents(const char *hex_contents) {
    size_t expected_size;
    uint8_t *const expected_contents = memfault_hex2bin(hex_contents, &expected_size);
    const int fd = open(tmp_queue_file, O_RDONLY);
    uint8_t *actual_contents = (uint8_t *)malloc(expected_size);
    CHECK_EQUAL(expected_size, read(fd, actual_contents, expected_size));
    MEMCMP_EQUAL(expected_contents, actual_contents, expected_size);
    CHECK_TRUE_TEXT(0 == read(fd, actual_contents, 1),
                    "Queue file contained more data than expected");
    close(fd);
    free(expected_contents);
    free(actual_contents);
  }

  static void read_and_complete_head(sMemfaultdQueue * queue) {
    uint32_t payload_size;
    free(memfaultd_queue_read_head(queue, &payload_size));
    memfaultd_queue_complete_read(queue);
  }
};

TEST_GROUP_BASE(TestGroup_Init, MemfaultdQueueUtest){};

// Tests that when a queue is created with an invalid file path, the queue falls back to using a
// memory-backed queue instead of a file-backed queue.
TEST(TestGroup_Init, Test_BadQueueFileFallBackToInMemoryQueue) {
  expect_queue_file_get_string_call("");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  CHECK(queue);
  CHECK(!memfaultd_queue_is_file_backed(queue));
  memfaultd_queue_destroy(queue);
}

// Tests that when a queue is created with a valid file path and the file does not exists, it gets
// created.
TEST(TestGroup_Init, Test_NewFileQueue) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  CHECK(queue);
  CHECK(memfaultd_queue_is_file_backed(queue));

  CHECK_EQUAL(0, access(tmp_queue_file, F_OK));
  memfaultd_queue_destroy(queue);
}
// Tests that when the queue file is full of unsent data and the end pointer is hit, the write
// pointer is set to the beginning of the file.
TEST(TestGroup_Init, Test_WritePointerSetToZeroWhenEndPointerIsHit) {
  create_queue_file("0000000000000000000000000000000000000000000000000000000000000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 32);
  const uint8_t payload[] = {0xFF};
  memfaultd_queue_write(queue, payload, sizeof(payload));

  memfaultd_queue_destroy(queue);
}

// Tests that when configured queue_size is too small, it falls back to using the default.
TEST(TestGroup_Init, Test_QueueSizeTooSmall) {
  expect_queue_file_get_string_call("");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 15);
  CHECK_EQUAL(1024 * 1024, memfaultd_queue_get_size(queue));
  memfaultd_queue_destroy(queue);
}

// Tests that when configured queue_size is too large, it falls back to using the default.
TEST(TestGroup_Init, Test_QueueSizeTooLarge) {
  expect_queue_file_get_string_call("");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, (1024 * 1024 * 1024) + 4);
  CHECK_EQUAL(1024 * 1024, memfaultd_queue_get_size(queue));
  memfaultd_queue_destroy(queue);
}

// Tests that when configured queue_size is unaligned, it rounds down to the nearest aligned size.
TEST(TestGroup_Init, Test_QueueSizeNotAligned) {
  expect_queue_file_get_string_call("");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 17);
  CHECK_EQUAL(16, memfaultd_queue_get_size(queue));
  memfaultd_queue_destroy(queue);
}

TEST_GROUP_BASE(TestGroup_InitFindPointers, MemfaultdQueueUtest){};

// Tests that the read/write/prev pointers are initialized correctly when using an empty queue file.
TEST(TestGroup_InitFindPointers, Test_NewFile) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  CHECK(queue);
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// containing an unread message at the start and extra space after it:
TEST(TestGroup_InitFindPointers, Test_OneUnreadMessageAtStartAndExtraSpaceAtEnd) {
  create_queue_file("A5014800000000000100000011000000"
                    "00000000000000000000000000000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 32);
  CHECK(queue);
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// containing a read message, an unread message and extra space after it:
TEST(TestGroup_InitFindPointers, Test_OneUnreadMessageAfterOneReadMessageAndExtraSpaceAtEnd) {
  create_queue_file("A5014801000000000100000011000000"
                    "A5010100000000000100000022000000"
                    "00000000000000000000000000000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 48);
  CHECK(queue);
  CHECK_EQUAL(4, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(8, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// containing a read message, an unread message and no extra space after it:
TEST(TestGroup_InitFindPointers, Test_OneUnreadMessageAfterOneReadMessageAndNoSpaceAtEnd) {
  create_queue_file("A5014801000000000100000011000000"
                    "A5010100000000000100000022000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 32);
  CHECK(queue);
  CHECK_EQUAL(4, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// containing an unread message at the beginning and an unread message at the end of the queue:
TEST(TestGroup_InitFindPointers, Test_UnreadWrapAround) {
  // Queue contains:
  // - unread
  // - read,
  // - unread
  create_queue_file("A5010200080000000100000044000000"
                    "A5010101000000000100000022000000"
                    "A5014900040000000100000033000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 48);
  CHECK(queue);
  CHECK_EQUAL(8, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// which was truncated, causing a previous_header pointer to be invalid:
TEST(TestGroup_InitFindPointers, Test_TruncatedFileWithWrapAround) {
  // Queue contains:
  // - unread (previous_header: 8 is out of bounds)
  // - read
  create_queue_file("A5010200080000000100000044000000"
                    "A5010101000000000100000022000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 32);
  CHECK(queue);
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// in which all messages are all read and exactly fill up the queue (no END_POINTER):
TEST(TestGroup_InitFindPointers, Test_AllRead) {
  // Queue contains:
  // - read (3x)
  create_queue_file("A5010201080000000100000044000000"
                    "A5010101000000000100000022000000"
                    "A5014901040000000100000033000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 48);
  CHECK(queue);
  // Note: we cannot tell where the read/write pointer is in this case and default to 0. This is
  // a shortcoming of the design of the format:
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(8, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are initialized correctly when using a queue file
// which was truncated in the middle of a message:
TEST(TestGroup_InitFindPointers, Test_TruncatedFileWithinMessage) {
  // Queue contains:
  // - read
  // - broken/truncated message (orignally unread)
  create_queue_file("A5010201000000000100000044000000"
                    "A5010100000000000100000022000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 28);
  CHECK(queue);
  CHECK_EQUAL(4, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that when the queue is completely full with unread messages, the read and write pointers
// are both initialized to 0:
TEST(TestGroup_InitFindPointers, Test_AllUnread) {
  // Queue contains:
  // - unread (3x)
  create_queue_file("A5010200080000000100000044000000"
                    "A5010100000000000100000022000000"
                    "A5014900040000000100000033000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 48);
  CHECK(queue);
  // Note: we cannot tell where the read/write pointer is in this case and default to 0. This is
  // a shortcoming of the design of the format:
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(8, memfaultd_queue_get_prev_ptr(queue));

  memfaultd_queue_destroy(queue);
}

struct MemfaultdQueueWriteUtest : MemfaultdQueueUtest {
  void test_write_move_read_pointer(size_t payload_size, uint32_t expected_read_ptr);
};

TEST_GROUP_BASE(TestGroup_Write, MemfaultdQueueWriteUtest){};

TEST(TestGroup_Write, Test_SimpleWriteIntoNewQueueFile) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 32);
  const uint8_t payload[] = {0xFF};
  memfaultd_queue_write(queue, payload, sizeof(payload));

  check_queue_file_contents("A5014F000000000001000000FF000000"
                            "00000000000000000000000000000000");
  memfaultd_queue_destroy(queue);
}

// Tests that when a payload is attempted to be written that is larger than the queue can ever
// contain, it is dropped and the file is not modified.
TEST(TestGroup_Write, Test_WriteLargerThanQueue) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  // This won't fit: the queue is 16 bytes, but the item is 12 + 8 (header + payload) = 20 bytes.
  const uint8_t payload[8] = {0};
  memfaultd_queue_write(queue, payload, sizeof(payload));

  // File is untouched:
  check_queue_file_contents("00000000000000000000000000000000");
  memfaultd_queue_destroy(queue);
}

TEST(TestGroup_Write, Test_WriteFitsExactly) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  uint8_t payload[4] = {0};
  memset(payload, 0x22, sizeof(payload));
  memfaultd_queue_write(queue, payload, sizeof(payload));

  check_queue_file_contents("A5017400000000000400000022222222");
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that when a payload is written that extends past the end of the queue, an END_POINTER is
// written and the payload is written at the beginning of the queue.
TEST(TestGroup_Write, Test_WriteWrapAround) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 32);
  // Payload one will be 12 + 8 = 20 bytes:
  uint8_t payload_one[8];
  memset(payload_one, 0x22, sizeof(payload_one));
  memfaultd_queue_write(queue, payload_one, sizeof(payload_one));

  // Payload takes 16 bytes. The call causes an END_POINTER (A55AA55A) to be written, wrap around
  // the write pointer to the beginning and then overwrite the first 16 bytes of the queue:
  const uint8_t payload_two = 0x11;
  memfaultd_queue_write(queue, &payload_two, 1);

  check_queue_file_contents("A5014800000000000100000011000000"
                            "22222222A55AA55A0000000000000000");
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(4, memfaultd_queue_get_write_ptr(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that the read/write/prev pointers are reinitialized correctly after a reset:
TEST(TestGroup_InitFindPointers, Test_PopulatedQueueThenReset) {
  create_queue_file("A5014801000000000100000011000000"
                    "A5010100000000000100000022000000"
                    "00000000000000000000000000000000");

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 48);
  CHECK(queue);
  memfaultd_queue_reset(queue);
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_prev_ptr(queue));

  check_queue_file_contents("00000000000000000000000011000000"
                            "A5010100000000000100000022000000"
                            "00000000000000000000000000000000");

  memfaultd_queue_destroy(queue);
}

void MemfaultdQueueWriteUtest::test_write_move_read_pointer(size_t payload_size,
                                                            uint32_t expected_read_ptr) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 64);
  for (int i = 0; i < 4; ++i) {
    // Payload takes 16 bytes. The call causes an END_POINTER (A55AA55A) to be written, wrap around
    // the write pointer to the beginning and then overwrite the first 16 bytes of the queue:
    const uint8_t payload_small = 0x11 * (i + 1);
    memfaultd_queue_write(queue, &payload_small, 1);
  }
  read_and_complete_head(queue);

  // Next write will happen before the read pointer:
  CHECK_EQUAL(16 / sizeof(uint32_t), memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL(0, memfaultd_queue_get_write_ptr(queue));

  // Read the next message (will be dropped before it's marked read):
  uint32_t p;
  free(memfaultd_queue_read_head(queue, &p));

  // Payload will be 12 (header) + payload_size bytes:
  uint8_t payload_big[payload_size];
  memset(payload_big, 0xAA, sizeof(payload_big));
  memfaultd_queue_write(queue, payload_big, sizeof(payload_big));

  CHECK_EQUAL(expected_read_ptr, memfaultd_queue_get_read_ptr(queue));
  CHECK_EQUAL((12 + payload_size) / sizeof(uint32_t), memfaultd_queue_get_write_ptr(queue));

  // Message was already removed from the queue:
  CHECK_FALSE(memfaultd_queue_complete_read(queue));

  memfaultd_queue_destroy(queue);
}

// Tests that when a payload is written and the read pointer would be overwritten, the read pointer
// is moved up to the next message until it is no longer overwritten ("dropping" oldest messages).
TEST(TestGroup_Write, Test_WriteMoveReadPointer) {
  const size_t payload_size = 32;
  const uint32_t expected_read_ptr = (3 * 16) / sizeof(uint32_t);
  test_write_move_read_pointer(payload_size, expected_read_ptr);
}

// Tests that when a payload is written and the read pointer would be overwritten, the read pointer
// is moved up to the next message until it wraps around.
TEST(TestGroup_Write, Test_WriteMoveReadPointerWrapAround) {
  const size_t payload_size = 40;
  const uint32_t expected_read_ptr = 0;
  test_write_move_read_pointer(payload_size, expected_read_ptr);
}

TEST(TestGroup_Write, Test_WritePreviousHeaderPointer) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 52);
  for (int i = 0; i < 4; ++i) {
    // Payload takes 16 bytes:
    const uint8_t payload_small = 0x11 * (i + 1);
    memfaultd_queue_write(queue, &payload_small, 1);
  }
  // Note: previous header indices are: 8 (due to the wrap-around), 0, 4
  check_queue_file_contents("A5010200080000000100000044000000"
                            "A5010100000000000100000022000000"
                            "A5014900040000000100000033000000"
                            // END POINTER:
                            "A55AA55A");
  memfaultd_queue_destroy(queue);
}

TEST(TestGroup_Write, Test_WriteZeroLengthPayload) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  uint8_t payload_zero_length[0];
  CHECK_FALSE(memfaultd_queue_write(queue, payload_zero_length, sizeof(payload_zero_length)));
  memfaultd_queue_destroy(queue);
}

TEST(TestGroup_Write, Test_WriteNullPayload) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  CHECK_FALSE(memfaultd_queue_write(queue, NULL, 1));
  memfaultd_queue_destroy(queue);
}

TEST_GROUP_BASE(TestGroup_Read, MemfaultdQueueUtest){};

TEST(TestGroup_Read, Test_ReadEmptyQueue) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);
  uint32_t payload_size;
  POINTERS_EQUAL(NULL, memfaultd_queue_read_head(queue, &payload_size));
  memfaultd_queue_destroy(queue);
}

TEST(TestGroup_Read, Test_ReadAndMarkRead) {
  expect_queue_file_get_string_call(tmp_queue_file);

  sMemfaultdQueue *queue = memfaultd_queue_init(g_stub_memfaultd, 16);

  const uint8_t payload_small = 0x11;
  memfaultd_queue_write(queue, &payload_small, 1);
  CHECK_EQUAL(0, memfaultd_queue_get_read_ptr(queue));

  uint32_t payload_size;
  uint8_t *payload = memfaultd_queue_read_head(queue, &payload_size);
  CHECK_TRUE(!!payload);
  MEMCMP_EQUAL(&payload_small, payload, 1);

  check_queue_file_contents("A5014800000000000100000011000000");
  CHECK_TRUE(memfaultd_queue_complete_read(queue));
  check_queue_file_contents("A5014801000000000100000011000000");
  CHECK_FALSE(memfaultd_queue_complete_read(queue));

  // Nothing to read any more -- read_ptr == write_ptr, but message is already read.
  POINTERS_EQUAL(NULL, memfaultd_queue_read_head(queue, &payload_size));

  free(payload);
  memfaultd_queue_destroy(queue);
}
