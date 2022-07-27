//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! File-backed transmit queue management implementation
//!

#include "queue.h"

#include <fcntl.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "memfaultd.h"

typedef struct MemfaultdQueue {
  sMemfaultd *memfaultd;
  //! @brief True if the buf memory is backed by a memory mapped file, false if it is backed by
  //! a heap-allocated buffer.
  bool is_file_backed;
  //! @brief Size of buf in bytes.
  int size;
  //! @brief The queue buffer.
  uint32_t *buf;
  //! @brief Index to the oldest, unread message in the queue. Should *always* point to a
  //! valid, unread sMemfaultQueueMsgHeader (never to garbage or END_POINTER), unless when read_ptr
  //! is equal to write_ptr, there is no data to read *or* the queue is entirely full.
  uint32_t read_ptr;
  //! @brief Index where to write the next message in the queue. This can potentially be pointing
  //! to an unread message in case the queue is entirely full.
  uint32_t write_ptr;
  //! @brief Index of the previously written message.
  uint32_t prev_ptr;
  //! @brief Flag to indicate that a memfaultd_queue_read_head() happened and a
  //! memfaultd_queue_complete_read() is expected to follow next. In case the read pointer is moved
  //! before the memfaultd_queue_complete_read(), the flag will be set to false.
  bool can_complete_read;
  pthread_mutex_t lock;
} sMemfaultdQueue;

/**
 * Message format, packed structure containing:
 * uint32_t  flags :
 *           uint8_t  magic number, 0xa5
 *           uint8_t  version number
 *           uint8_t  crc8 of payload data (excl. padding bytes)
 *           uint8_t  flags:
 *                    0x01  message read
 * uint32_t  previous header
 * uint32_t  payload size (in bytes)
 * uint8_t[] payload data, padded to 4-byte boundary with 0x00 bytes
 */
typedef struct MemfaultQueueMsgHeader {
  uint8_t magic;
  uint8_t version;
  uint8_t crc;
  uint8_t flags;
  uint32_t prev_header;
  uint32_t payload_size_bytes;
  uint8_t payload[];
} sMemfaultQueueMsgHeader;

_Static_assert(sizeof(sMemfaultQueueMsgHeader) == 12, "MemfaultQueueMsgHeader size mismatch");

#define HEADER_LEN (sizeof(sMemfaultQueueMsgHeader) / sizeof(uint32_t))

#define HEADER_MAGIC_NUMBER 0xa5u
#define HEADER_VERSION_NUMBER 0x01
#define HEADER_FLAGS_FLAG_READ_MASK (1 << 0)

#define END_POINTER 0x5aa55aa5

#define QUEUE_SIZE_MIN (sizeof(sMemfaultQueueMsgHeader) + 4)
#define QUEUE_SIZE_MAX (1024 * 1024 * 1024)
#define QUEUE_SIZE_ALIGNMENT 4

/**
 * @brief Calculates CRC8 of data
 *
 * @param data Pointer to memory
 * @param len Length of data
 * @return uint8_t returned CRC8
 */
static uint8_t prv_queue_crc8(const void *data, uint32_t len) {
  const uint8_t *ptr = data;
  uint8_t crc = 0x00;
  for (int i = 0; i < len; ++i) {
    crc ^= ptr[i];
    for (int j = 0; j < 8; ++j) {
      if (crc & 1) {
        crc ^= 0x91;
      }
      crc >>= 1;
    }
  }
  return crc;
}

static uint32_t prv_bytes_to_words_round_up(uint32_t size_bytes) {
  return (size_bytes + 3) / sizeof(uint32_t);
}

/**
 * @brief Get pointer of next message, wrapping around to the start when the END_POINTER or end of
 * the buffer has been reached.
 *
 * @param handle Queue handle
 * @param ptr Current pointer
 * @return uint32_t New pointer
 */
static uint32_t prv_get_next_message(sMemfaultdQueue *handle, uint32_t ptr) {
  sMemfaultQueueMsgHeader *header = (sMemfaultQueueMsgHeader *)&handle->buf[ptr];
  const uint32_t next_ptr =
    ptr + HEADER_LEN + prv_bytes_to_words_round_up(header->payload_size_bytes);
  if (next_ptr >= handle->size / sizeof(uint32_t)) {
    // Wrapped around end of queue
    return 0;
  }
  if (handle->buf[next_ptr] == END_POINTER) {
    // Reached end pointer
    return 0;
  }
  return next_ptr;
}

/**
 * @brief Checks whether the message's payload size is within the bounds of the queue's buffer
 * @param handle Queue handle
 * @param header Pointer to header
 * @return true if payload size is within bounds, false otherwise
 */
static bool prv_is_msg_in_bounds(sMemfaultdQueue *handle, const sMemfaultQueueMsgHeader *header) {
  return ((uint8_t *)header >= (uint8_t *)handle->buf) &&
         (((uint8_t *)header) + sizeof(*header) + header->payload_size_bytes <
          ((uint8_t *)handle->buf) + handle->size);
}

static bool prv_is_msg_read(const sMemfaultQueueMsgHeader *header) {
  return header->flags & HEADER_FLAGS_FLAG_READ_MASK;
}

/**
 * @brief Validates a message
 *
 * @param handle Queue handle
 * @param ptr Pointer to message structure
 * @return true Message at ptr is valid
 * @return false No valid
 */
static bool prv_is_msg_valid(sMemfaultdQueue *handle, const sMemfaultQueueMsgHeader *header) {
  if (!prv_is_msg_in_bounds(handle, header)) {
    return false;
  }

  if (header->magic != HEADER_MAGIC_NUMBER) {
    return false;
  }

  const uint8_t crc = prv_queue_crc8(&header->payload, header->payload_size_bytes);
  if (crc != header->crc) {
    return false;
  }
  return true;
}

/**
 * @brief Find read & write pointers at start of day
 *
 * @param handle Queue handle
 */
static void prv_queue_find_read_write_ptr(sMemfaultdQueue *handle) {
  /*
   * Start by setting the write_ptr, read_ptr and prev_ptr to index 0.
   * Then, find the write_ptr by moving forwards until we find either:
   * - Next message is invalid. If all messages were read so far, move read_ptr too.
   * - Move from an unread block to a read block (perfectly aligned wrap)
   * - Wrapped around (hit the end of the buffer or END_POINTER)
   * In this loop, if we move from a read block to an unread block, set the read_ptr.
   * Finally, if the read_ptr is still 0, walk backwards to see if there are more unread
   * messages after the wrap-around.
   */
  uint32_t tmp = 0;
  uint32_t read_ptr = 0;
  uint32_t write_ptr = 0;
  uint32_t prev_ptr = 0;
  bool last_read = true;
  while (true) {
    sMemfaultQueueMsgHeader *header = (sMemfaultQueueMsgHeader *)&handle->buf[tmp];
    if (!prv_is_msg_valid(handle, header)) {
      write_ptr = tmp;
      if (last_read) {
        // All messages up until now were read, move the read_ptr too:
        read_ptr = tmp;
      }
      break;
    }

    const bool is_msg_read = prv_is_msg_read(header);
    if (is_msg_read && !last_read) {
      write_ptr = tmp;
      break;
    } else if (!is_msg_read && last_read) {
      read_ptr = tmp;
    }
    last_read = is_msg_read;

    prev_ptr = tmp;
    tmp = prv_get_next_message(handle, tmp);
    if (tmp == 0) {
      // Wrapped around!
      // Note: in this edge case, we can't reliably determine the write pointer and defaulting to 0!
      write_ptr = tmp;
      break;
    }
  }

  // See if there's more unread messages after the wrap around:
  if (read_ptr == 0 && write_ptr != 0) {
    do {
      sMemfaultQueueMsgHeader *header = (sMemfaultQueueMsgHeader *)&handle->buf[read_ptr];
      sMemfaultQueueMsgHeader *prev_header =
        (sMemfaultQueueMsgHeader *)&handle->buf[header->prev_header];
      if (header == prev_header) {
        // Initial message's prev_header points to itself.
        break;
      }
      if (!prv_is_msg_valid(handle, prev_header)) {
        break;
      }
      if (prv_is_msg_read(prev_header)) {
        break;
      }
      read_ptr = header->prev_header;
    } while (read_ptr > 0);  // failsafe
  }

  handle->read_ptr = read_ptr;
  handle->write_ptr = write_ptr;
  handle->prev_ptr = prev_ptr;
}

static bool prv_check_queue_size(int *queue_size) {
  if (*queue_size % QUEUE_SIZE_ALIGNMENT != 0) {
    const int aligned_queue_size = (*queue_size / QUEUE_SIZE_ALIGNMENT) * QUEUE_SIZE_ALIGNMENT;
    fprintf(stderr, "queue:: queue_size (%i) must be a multiple of 4. Rounding down to %i.\n",
            *queue_size, aligned_queue_size);
    *queue_size = aligned_queue_size;
  }
  if (*queue_size < QUEUE_SIZE_MIN) {
    fprintf(stderr,
            "queue:: queue_size (%i) too small, minimum size is %lu. Falling back to default "
            "size.\n",
            *queue_size, QUEUE_SIZE_MIN);
    return false;
  }
  if (*queue_size > QUEUE_SIZE_MAX) {
    fprintf(
      stderr,
      "queue:: queue_size (%i) too large, maximum size is %i. Falling back to default size.\n",
      *queue_size, QUEUE_SIZE_MAX);
    return false;
  }
  return true;
}

/**
 * @brief Initialises the queue object
 *
 * @param memfaultd Main memfaultd handle
 * @return memfaultd_queue_h queue object
 */
sMemfaultdQueue *memfaultd_queue_init(sMemfaultd *memfaultd, int size) {
  sMemfaultdQueue *handle = calloc(sizeof(sMemfaultdQueue), 1);

  handle->memfaultd = memfaultd;
  handle->size = size;

  if (pthread_mutex_init(&handle->lock, NULL) != 0) {
    fprintf(stderr, "queue:: Failed to initialise queue mutex.\n");
    free(handle);
    return NULL;
  }

  if (!prv_check_queue_size(&handle->size)) {
    /* Default to 1MiB */
    handle->size = 1024 * 1024;
  }

  char *queue_file = memfaultd_generate_rw_filename(handle->memfaultd, "queue");

  int fd = -1;
  if (queue_file) {
    if ((fd = open(queue_file, O_RDWR | O_CREAT, S_IRUSR | S_IWUSR)) == -1) {
      fprintf(stderr, "queue:: Failed to open '%s', falling back to non-persistent queue.\n",
              queue_file);
    } else {
      if (ftruncate(fd, handle->size) == -1) {
        close(fd);
        fd = -1;
        fprintf(stderr, "queue:: Failed to resize '%s', falling back to non-persistent queue.\n",
                queue_file);
      } else {
        if ((handle->buf = mmap(NULL, handle->size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0)) ==
            (void *)-1) {
          close(fd);
          fd = -1;
          fprintf(stderr, "queue:: Failed to mmap '%s', falling back to non-persistent queue.\n",
                  queue_file);
        } else {
          close(fd);
        }
      }
    }
    free(queue_file);
  }
  if (fd == -1) {
    handle->buf = calloc(handle->size, 1);
    handle->is_file_backed = false;
  } else {
    handle->is_file_backed = true;
  }

  prv_queue_find_read_write_ptr(handle);

  return handle;
}

/**
 * @brief Destroys the queue handle
 *
 * @param handle Queue handle
 */
void memfaultd_queue_destroy(sMemfaultdQueue *handle) {
  if (handle) {
    if (handle->is_file_backed) {
      munmap(handle->buf, handle->size);
    } else if (handle->buf) {
      free(handle->buf);
    }

    pthread_mutex_destroy(&handle->lock);
    free(handle);
  }
}

/**
 * @brief Resets the internal queue state to empty
 *
 * @param handle Queue handle
 */
void memfaultd_queue_reset(sMemfaultdQueue *handle) {
  pthread_mutex_lock(&handle->lock);
  handle->read_ptr = 0;
  handle->write_ptr = 0;
  handle->prev_ptr = 0;

  memset(handle->buf, 0, HEADER_LEN * sizeof(uint32_t));
  msync(handle->buf, HEADER_LEN * sizeof(uint32_t), MS_SYNC);

  pthread_mutex_unlock(&handle->lock);
}

/**
 * @brief Returns copies of head of the queue
 *
 * @param handle Queue handle
 * @param[out] payload Payload string
 * @param[out] payload_size_bytes Payload size in bytes
 * @return Pointer to heap-allocated message found on head of queue, or NULL if queue is empty or
 * memory failed to be allocated.
 */
uint8_t *memfaultd_queue_read_head(sMemfaultdQueue *handle, uint32_t *payload_size_bytes) {
  uint8_t *payload = NULL;
  pthread_mutex_lock(&handle->lock);

  sMemfaultQueueMsgHeader *header = (sMemfaultQueueMsgHeader *)&handle->buf[handle->read_ptr];
  if (handle->read_ptr == handle->write_ptr) {
    if (!prv_is_msg_valid(handle, header) || prv_is_msg_read(header)) {
      // read_ptr is caught up, nothing to read!
      goto unlock;
    }
  }

  payload = malloc(header->payload_size_bytes);
  if (!payload) {
    return payload;
  }
  memcpy(payload, header->payload, header->payload_size_bytes);
  *payload_size_bytes = header->payload_size_bytes;

  // Allow a memfaultd_queue_complete_read() call now:
  handle->can_complete_read = true;

unlock:
  pthread_mutex_unlock(&handle->lock);
  return payload;
}

/**
 * @brief Removes message from head of the queue
 *
 * @param handle Queue handle
 * @return true if a message was removed, false if not
 */
bool memfaultd_queue_complete_read(sMemfaultdQueue *handle) {
  pthread_mutex_lock(&handle->lock);

  if (!handle->can_complete_read) {
    pthread_mutex_unlock(&handle->lock);
    return false;
  }

  sMemfaultQueueMsgHeader *header = (sMemfaultQueueMsgHeader *)&handle->buf[handle->read_ptr];
  header->flags |= HEADER_FLAGS_FLAG_READ_MASK;
  msync(&header->flags, sizeof(header->flags), MS_SYNC);

  handle->read_ptr = prv_get_next_message(handle, handle->read_ptr);

  // Flip to false, to make another ..complete_read() call -- before a ..read_head() call -- bail:
  handle->can_complete_read = false;

  pthread_mutex_unlock(&handle->lock);
  return true;
}

/**
 * @brief Adds message to queue
 *
 * @param handle Queue handle
 * @param payload Payload data
 * @param payload_size_bytes Payload size in bytes
 * @return true Successfully added message to queue
 * @return false Failed to add
 */
bool memfaultd_queue_write(sMemfaultdQueue *handle, const uint8_t *payload,
                           uint32_t payload_size_bytes) {
  if (payload_size_bytes == 0 || payload == NULL) {
    return false;
  }

  pthread_mutex_lock(&handle->lock);

  const uint32_t payload_padded_size_words = prv_bytes_to_words_round_up(payload_size_bytes);
  const uint32_t message_size_words = HEADER_LEN + payload_padded_size_words;
  if (message_size_words > handle->size / sizeof(uint32_t)) {
    fprintf(stderr, "queue:: payload size %u bytes is too large for queue size %u bytes.\n",
            payload_size_bytes, handle->size);
    pthread_mutex_unlock(&handle->lock);
    return false;
  }

  uint32_t *ptr = &handle->buf[handle->write_ptr];
  const bool read_ptr_equals_write_ptr = (handle->read_ptr == handle->write_ptr);

  if (handle->write_ptr + message_size_words > handle->size / sizeof(uint32_t)) {
    // Message is too big, add end marker and loop back around to start
    *ptr = END_POINTER;
    msync(ptr, sizeof(uint32_t), MS_SYNC);
    handle->write_ptr = 0;
    ptr = &handle->buf[0];
  }

  const uint32_t write_end = handle->write_ptr + message_size_words;
  const uint32_t next_write_ptr = write_end % (handle->size / sizeof(uint32_t));

  if (read_ptr_equals_write_ptr) {
    // Note: either the read_ptr is caught up, or queue is entirely full and the write_ptr caught
    // up with the read_ptr (wrapping around). In both cases, we'll move the read_ptr along with
    // the new write:
    handle->read_ptr = handle->write_ptr;
  } else if (handle->read_ptr > handle->write_ptr && handle->read_ptr < write_end) {
    // Read pointer is after write pointer and new message will overwrite read pointer, move it
    // forwards:
    uint32_t read_ptr = handle->read_ptr;
    while (true) {
      read_ptr = prv_get_next_message(handle, read_ptr);
      const bool did_wrap = (read_ptr == 0);
      if (read_ptr >= write_end || did_wrap) {
        handle->read_ptr = read_ptr;
        break;
      }
    }

    // In case memfaultd_queue_read_head() had just been called, flag to avoid marking the wrong
    // message as sent in a subsequent memfaultd_queue_complete_read() call:
    handle->can_complete_read = false;
  }

  sMemfaultQueueMsgHeader *const header = (sMemfaultQueueMsgHeader *)ptr;
  *header = (sMemfaultQueueMsgHeader){
    .magic = HEADER_MAGIC_NUMBER,
    .version = HEADER_VERSION_NUMBER,
    .crc = prv_queue_crc8(payload, payload_size_bytes),
    .flags = 0,
    .prev_header = handle->prev_ptr,
    .payload_size_bytes = payload_size_bytes,
  };
  memcpy(header->payload, payload, payload_size_bytes);

  // Zero-out padding bytes:
  const size_t padding_size_bytes =
    payload_padded_size_words * sizeof(uint32_t) - payload_size_bytes;
  if (padding_size_bytes > 0) {
    memset(header->payload + payload_size_bytes, 0, padding_size_bytes);
  }

  msync(header, message_size_words * sizeof(uint32_t), MS_SYNC);

  handle->prev_ptr = handle->write_ptr;
  handle->write_ptr = next_write_ptr;

  pthread_mutex_unlock(&handle->lock);
  return true;
}

#ifdef MEMFAULT_UNITTEST

bool memfaultd_queue_is_file_backed(sMemfaultdQueue *handle) { return handle->is_file_backed; }
int memfaultd_queue_get_size(sMemfaultdQueue *handle) { return handle->size; }
uint32_t memfaultd_queue_get_read_ptr(sMemfaultdQueue *handle) { return handle->read_ptr; }
uint32_t memfaultd_queue_get_write_ptr(sMemfaultdQueue *handle) { return handle->write_ptr; }
uint32_t memfaultd_queue_get_prev_ptr(sMemfaultdQueue *handle) { return handle->prev_ptr; }

#endif
