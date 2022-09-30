//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Memory buffer based IO implementations.

#include "core_elf_memory_io.h"

#include <string.h>

#include "memfault/core/math.h"

static ssize_t prv_mio_read(const struct MemfaultCoreElfReadIO *io, void *buffer,
                            size_t buffer_size) {
  sMemfaultCoreElfReadMemoryIO *mio = (sMemfaultCoreElfReadMemoryIO *)io;
  const ssize_t size_to_read = MEMFAULT_MIN((ssize_t)buffer_size, mio->end - mio->cursor);
  const ssize_t size = MEMFAULT_MIN(size_to_read, mio->next_read_size);
  if (buffer != NULL) {
    memcpy(buffer, mio->cursor, size);
  }
  mio->cursor += size;
  return size;
}

void memfault_core_elf_read_memory_io_init(sMemfaultCoreElfReadMemoryIO *mio, uint8_t *buffer,
                                           size_t buffer_size) {
  *mio = (sMemfaultCoreElfReadMemoryIO){
    .io =
      {
        .read = prv_mio_read,
      },
    .buffer = buffer,
    .end = buffer + buffer_size,
    .cursor = buffer,
    // Default to reading one byte at a time:
    .next_read_size = 1,
  };
}

static ssize_t prv_mio_write(sMemfaultCoreElfWriteIO *io, const void *data, size_t size) {
  sMemfaultCoreElfWriteMemoryIO *mio = (sMemfaultCoreElfWriteMemoryIO *)io;
  if (mio->cursor + size > mio->end) {
    return -1;
  }
  memcpy(mio->cursor, data, size);
  mio->cursor += size;
  return (ssize_t)size;
}

static bool prv_mio_sync(const sMemfaultCoreElfWriteIO *io) { return true; }

void memfault_core_elf_write_memory_io_init(sMemfaultCoreElfWriteMemoryIO *mio, void *buffer,
                                            size_t buffer_size) {
  *mio = (sMemfaultCoreElfWriteMemoryIO){
    .io =
      {
        .write = prv_mio_write,
        .sync = prv_mio_sync,
      },
    .buffer = buffer,
    .end = ((uint8_t *)buffer) + buffer_size,
    .cursor = (uint8_t *)buffer,
  };
}
