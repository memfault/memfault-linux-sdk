#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! ELF coredump transformer

#include <sys/types.h>

#include "core_elf_metadata.h"
#include "core_elf_reader.h"
#include "core_elf_writer.h"

#ifdef __cplusplus
extern "C" {
#endif

#define MEMFAULT_CORE_ELF_TRANSFORMER_PROC_MEM_COPY_BUFFER_SIZE_BYTES (4 * 1024)

typedef struct MemfaultCoreElfTransformerHandler sMemfaultCoreElfTransformerHandler;

/**
 * Interface of an object that handles callbacks from sMemfaultCoreElfTransformer.
 */
typedef struct MemfaultCoreElfTransformerHandler {
  /**
   * Callback to copy process memory to a buffer.
   * @param handler The handler itself.
   * @param vaddr Virtual address to copy from.
   * @param size Number of bytes to copy.
   * @param buffer Buffer to copy the data into.
   * @return The number of bytes copied, or -1 in case an error occurred. errno is expected to be
   * set with the error code.
   **/
  ssize_t (*copy_proc_mem)(sMemfaultCoreElfTransformerHandler *handler, Elf_Addr vaddr,
                           Elf64_Xword size, void *buffer);
} sMemfaultCoreElfTransformerHandler;

typedef struct MemfaultCoreElfTransformer {
  sMemfaultCoreElfReaderHandler read_handler;
  sMemfaultCoreElfReader reader;
  sMemfaultCoreElfWriter writer;
  const sMemfaultCoreElfMetadata *metadata;
  sMemfaultCoreElfTransformerHandler *transformer_handler;

  char *warnings[16];
  size_t next_warning_idx;

  bool write_success;
} sMemfaultCoreElfTransformer;

/**
 * Initializes a ELF coredump transformer.
 * @param transformer The transformer object.
 * @param reader_io The reader object to use to read the original coredump.
 * @param writer_io The writer object to use to write out the transformed coredump.
 * @param metadata The metadata to write into the ELF coredump.
 * @param transformer_handler Pointer to the handler that will handle callbacks from the
 * transformer.
 */
void memfault_core_elf_transformer_init(sMemfaultCoreElfTransformer *transformer,
                                        sMemfaultCoreElfReadIO *reader_io,
                                        sMemfaultCoreElfWriteIO *writer_io,
                                        const sMemfaultCoreElfMetadata *metadata,
                                        sMemfaultCoreElfTransformerHandler *transformer_handler);

/**
 * Runs the transformer, causing the ELF data from the reader to be read, transformed and written to
 * the writer.
 * @param transformer
 * @return True if the transform was successful, or false if not.
 */
bool memfault_core_elf_transformer_run(sMemfaultCoreElfTransformer *transformer);

/**
 * Transformer handler implementation that uses /proc/<pid>/mem to copy out LOAD segment data.
 */
typedef struct MemfaultCoreElfTransformerProcfsHandler {
  sMemfaultCoreElfTransformerHandler handler;
  int fd;
} sMemfaultCoreElfTransformerProcfsHandler;

/**
 * Initializes a sMemfaultCoreElfTransformerProcfsHandler.
 * @param handler The handler.
 * @return True if it was initialized successfully, or false if not.
 */
bool memfault_init_core_elf_transformer_procfs_handler(
  sMemfaultCoreElfTransformerProcfsHandler *handler, pid_t pid);

/**
 * Cleans up any resources used by the sMemfaultCoreElfTransformerProcfsHandler.
 * @param handler The handler.
 * @return True if it was deinitialized successfully, or false if not.
 */
bool memfault_deinit_core_elf_transformer_procfs_handler(
  sMemfaultCoreElfTransformerProcfsHandler *handler);

#ifdef __cplusplus
}
#endif
