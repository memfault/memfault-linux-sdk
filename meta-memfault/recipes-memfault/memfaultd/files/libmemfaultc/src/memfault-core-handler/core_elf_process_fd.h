#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Processes the coredump ELF stream from a file descriptor.

#include <stdbool.h>
#include <unistd.h>

#include "memfaultd.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MemfaultProcessCoredumpCtx {
  int input_fd;
  pid_t pid;

  sMemfaultdDeviceSettings *device_settings;
  const char *software_type;
  const char *software_version;

  const char *output_file;
  size_t max_size;
  bool gzip_enabled;
} sMemfaultProcessCoredumpCtx;

bool core_elf_process_fd(const sMemfaultProcessCoredumpCtx *ctx);

#ifdef __cplusplus
}
#endif
