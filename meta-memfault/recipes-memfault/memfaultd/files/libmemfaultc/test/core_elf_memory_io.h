#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Memory buffer based IO implementations.

#include "memfault-core-handler/core_elf_reader.h"
#include "memfault-core-handler/core_elf_writer.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MemfaultCoreElfReadMemoryIO {
  sMemfaultCoreElfReadIO io;
  uint8_t *buffer;
  uint8_t *end;
  uint8_t *cursor;
  ssize_t next_read_size;
} sMemfaultCoreElfReadMemoryIO;

void memfault_core_elf_read_memory_io_init(sMemfaultCoreElfReadMemoryIO *mio, uint8_t *buffer,
                                           size_t buffer_size);

typedef struct MemfaultCoreElfWriteMemoryIO {
  sMemfaultCoreElfWriteIO io;
  void *buffer;
  uint8_t *end;
  uint8_t *cursor;
} sMemfaultCoreElfWriteMemoryIO;

void memfault_core_elf_write_memory_io_init(sMemfaultCoreElfWriteMemoryIO *mio, void *buffer,
                                            size_t buffer_size);

#ifdef __cplusplus
}
#endif
