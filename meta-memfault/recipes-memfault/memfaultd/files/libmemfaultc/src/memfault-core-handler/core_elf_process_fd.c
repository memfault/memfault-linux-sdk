//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Processes the coredump ELF stream from a file descriptor.

#include "core_elf_process_fd.h"

#include <errno.h>
#include <fcntl.h>
#include <linux/limits.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#include "core_elf_transformer.h"
#include "memfault/util/version.h"

static void prv_log_coredump_request(int pid) {
  char cmdline_path[32];
  char cmdline[PATH_MAX] = "???";
  FILE *fd = NULL;
  snprintf(cmdline_path, sizeof(cmdline_path), "/proc/%d/cmdline", pid);

  if (!(fd = fopen(cmdline_path, "r"))) {
    goto cleanup;
  }

  if (fgets(cmdline, PATH_MAX, fd) == NULL) {
    goto cleanup;
  }
  cmdline[PATH_MAX - 1] = '\0';

cleanup:
  if (fd) {
    fclose(fd);
  }

  //! cmdline actually holds null-delimited list of the full command line, but we just want argv[0]
  fprintf(stderr, "coredump:: Received corefile for PID %d, process '%s'\n", pid, cmdline);
}

static void prv_init_metadata(const sMemfaultProcessCoredumpCtx *ctx,
                              sMemfaultCoreElfMetadata *metadata) {
  const time_t now_epoch_s = time(NULL);

  *metadata = (sMemfaultCoreElfMetadata){
    .linux_sdk_version = memfaultd_sdk_version,
    .captured_time_epoch_s = now_epoch_s != -1 ? now_epoch_s : 0,
    .device_serial = ctx->device_settings->device_id,
    .hardware_version = ctx->device_settings->hardware_version,
    .software_version = ctx->software_version,
    .software_type = ctx->software_type,
  };
}

static bool prv_transform_coredump_from_fd_to_file(const sMemfaultProcessCoredumpCtx *ctx) {
  sMemfaultCoreElfReadFileIO reader_io;
  sMemfaultCoreElfWriteFileIO writer_io;
  sMemfaultCoreElfWriteGzipIO gzip_io;
  bool gzip_io_initialized = false;
  sMemfaultCoreElfTransformer transformer;
  sMemfaultCoreElfMetadata metadata;
  sMemfaultCoreElfTransformerProcfsHandler transformer_handler;

  bool result = false;
  int out_fd = -1;

  prv_init_metadata(ctx, &metadata);

  if (!memfault_init_core_elf_transformer_procfs_handler(&transformer_handler, ctx->pid)) {
    goto cleanup;
  }

  if ((out_fd = open(ctx->output_file, O_WRONLY | O_CREAT | O_EXCL | O_TRUNC, S_IRUSR | S_IWUSR)) ==
      -1) {
    fprintf(stderr, "coredump:: Failed to open '%s'\n", ctx->output_file);
    goto cleanup;
  }

  memfault_core_elf_write_file_io_init(&writer_io, out_fd, ctx->max_size);
  if (ctx->gzip_enabled) {
    gzip_io_initialized = memfault_core_elf_write_gzip_io_init(&gzip_io, &writer_io.io);
    if (!gzip_io_initialized) {
      fprintf(stderr, "coredump:: Failed to init gzip io\n");
      goto cleanup;
    }
  }
  memfault_core_elf_read_file_io_init(&reader_io, ctx->input_fd);
  memfault_core_elf_transformer_init(&transformer, &reader_io.io,
                                     ctx->gzip_enabled ? &gzip_io.io : &writer_io.io, &metadata,
                                     &transformer_handler.handler);

  result = memfault_core_elf_transformer_run(&transformer);

cleanup:
  memfault_deinit_core_elf_transformer_procfs_handler(&transformer_handler);
  if (gzip_io_initialized) {
    memfault_core_elf_write_gzip_io_deinit(&gzip_io);
  }
  if (out_fd != -1) {
    close(out_fd);
  }
  return result;
}

bool core_elf_process_fd(const sMemfaultProcessCoredumpCtx *ctx) {
  prv_log_coredump_request(ctx->pid);

  fprintf(stderr, "coredump:: writing coredump with max size: %lu\n",
          (long unsigned int)ctx->max_size);
  if (!prv_transform_coredump_from_fd_to_file(ctx)) {
    if (unlink(ctx->output_file) == -1 && errno != ENOENT) {
      fprintf(stderr, "Failed to remove core file '%s' after failure : %s\n", ctx->output_file,
              strerror(errno));
    }
    return false;
  }

  return true;
}
