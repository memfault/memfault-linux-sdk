//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! coredump plugin implementation

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/un.h>
#include <unistd.h>
#include <uuid/uuid.h>

#include "core_elf_transformer.h"
#include "coredump_ratelimiter.h"
#include "memfault/core/math.h"
#include "memfault/util/disk.h"
#include "memfault/util/rate_limiter.h"
#include "memfault/util/string.h"
#include "memfault/util/version.h"
#include "memfaultd.h"

#define CORE_PATTERN_PATH "/proc/sys/kernel/core_pattern"
#define CORE_PATTERN "|/usr/sbin/memfault-core-handler %P"
#define COMPRESSION_DEFAULT "gzip"

struct MemfaultdPlugin {
  sMemfaultd *memfaultd;
  bool enable_data_collection;
  sMemfaultdRateLimiter *rate_limiter;
  char *core_dir;
  bool gzip_enabled;
};

static char *prv_create_dir(sMemfaultdPlugin *handle, const char *subdir) {
  const char *data_dir;
  if (!memfaultd_get_string(handle->memfaultd, "", "data_dir", &data_dir) ||
      strlen(data_dir) == 0) {
    fprintf(stderr, "coredump:: No data_dir defined\n");
    return NULL;
  }

  char *path;
  char *fmt = "%s/%s";
  if (memfault_asprintf(&path, fmt, data_dir, subdir) == -1) {
    fprintf(stderr, "coredump:: Failed to create path buffer\n");
    return NULL;
  }

  struct stat sb;
  if (stat(path, &sb) == 0 && S_ISDIR(sb.st_mode)) {
    return path;
  }

  if (mkdir(path, 0755) == -1) {
    fprintf(stderr, "coredump:: Failed to mkdir '%s'\n", path);
    free(path);
    return NULL;
  }
  return path;
}

static char *prv_generate_filename(sMemfaultdPlugin *handle, const char *prefix,
                                   const char *extension) {
  char *filename = NULL;

  uuid_t uuid;
  char uuid_str[37];
  uuid_generate(uuid);
  uuid_unparse_lower(uuid, uuid_str);

  char *fmt = "%s/%s%s%s";
  if (memfault_asprintf(&filename, fmt, handle->core_dir, prefix, uuid_str, extension) == -1) {
    fprintf(stderr, "coredump:: Failed to create filename buffer\n");
    goto cleanup;
  }

  return filename;

cleanup:
  free(filename);
  return NULL;
}

static bool prv_init_metadata(sMemfaultdPlugin *handle, sMemfaultCoreElfMetadata *metadata) {
  const char *software_type = NULL;
  const char *software_version = NULL;

  const sMemfaultdDeviceSettings *const device_settings =
    memfaultd_get_device_settings(handle->memfaultd);

  if (!memfaultd_get_string(handle->memfaultd, "", "software_type", &software_type) ||
      strlen(software_type) == 0) {
    fprintf(stderr, "coredump:: Failed to get software_type\n");
    return false;
  }

  if (!memfaultd_get_string(handle->memfaultd, "", "software_version", &software_version) ||
      strlen(software_version) == 0) {
    fprintf(stderr, "coredump:: Failed to get software_version\n");
    return false;
  }

  const time_t now_epoch_s = time(NULL);

  *metadata = (sMemfaultCoreElfMetadata){
    .linux_sdk_version = memfaultd_sdk_version,
    .captured_time_epoch_s = now_epoch_s != -1 ? now_epoch_s : 0,
    .device_serial = device_settings->device_id,
    .hardware_version = device_settings->hardware_version,
    .software_version = software_version,
    .software_type = software_type,
  };
  return true;
}

static bool prv_transform_coredump_from_fd_to_file(sMemfaultdPlugin *handle, const char *path,
                                                   int in_fd, pid_t pid, size_t max_size) {
  sMemfaultCoreElfReadFileIO reader_io;
  sMemfaultCoreElfWriteFileIO writer_io;
  sMemfaultCoreElfWriteGzipIO gzip_io;
  bool gzip_io_initialized = false;
  sMemfaultCoreElfTransformer transformer;
  sMemfaultCoreElfMetadata metadata;
  sMemfaultCoreElfTransformerProcfsHandler transformer_handler;

  bool result = false;
  int out_fd = -1;

  if (!prv_init_metadata(handle, &metadata)) {
    goto cleanup;
  }

  if (!memfault_init_core_elf_transformer_procfs_handler(&transformer_handler, pid)) {
    goto cleanup;
  }

  if ((out_fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_TRUNC, S_IRUSR | S_IWUSR)) == -1) {
    fprintf(stderr, "coredump:: Failed to open '%s'\n", path);
    goto cleanup;
  }

  memfault_core_elf_write_file_io_init(&writer_io, out_fd, max_size);
  if (handle->gzip_enabled) {
    gzip_io_initialized = memfault_core_elf_write_gzip_io_init(&gzip_io, &writer_io.io);
    if (!gzip_io_initialized) {
      fprintf(stderr, "coredump:: Failed to init gzip io\n");
      goto cleanup;
    }
  }
  memfault_core_elf_read_file_io_init(&reader_io, in_fd);
  memfault_core_elf_transformer_init(&transformer, &reader_io.io,
                                     handle->gzip_enabled ? &gzip_io.io : &writer_io.io, &metadata,
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

static sMemfaultdTxData *prv_build_queue_entry(bool gzip_enabled, const char *filename,
                                               uint32_t *payload_size) {
  size_t filename_len = strlen(filename);
  sMemfaultdTxData *data;
  if (!(data = malloc(sizeof(sMemfaultdTxData) + filename_len + 1))) {
    fprintf(stderr, "network:: Failed to create upload_request buffer\n");
    return NULL;
  }

  data->type =
    gzip_enabled ? kMemfaultdTxDataType_CoreUploadWithGzip : kMemfaultdTxDataType_CoreUpload;
  strcpy((char *)data->payload, filename);

  *payload_size = filename_len + 1;
  return data;
}

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

static size_t prv_check_for_available_space(sMemfaultdPlugin *handle) {
  size_t min_headroom = 0;
  size_t max_usage = 0;
  size_t max_size = 0;
  memfaultd_get_integer(handle->memfaultd, "coredump_plugin", "storage_min_headroom_kib",
                        (int *)&min_headroom);
  memfaultd_get_integer(handle->memfaultd, "coredump_plugin", "storage_max_usage_kib",
                        (int *)&max_usage);
  memfaultd_get_integer(handle->memfaultd, "coredump_plugin", "coredump_max_size_kib",
                        (int *)&max_size);

  if (min_headroom == 0 && max_usage == 0 && max_size == 0) {
    //! No limits, return non-privileged space left on device - leaves 5% reserve on ext[2-4]
    //! filesystems
    const bool privileged = false;
    return memfaultd_get_free_space(handle->core_dir, privileged);
  }

  min_headroom *= 1024;
  max_usage *= 1024;
  max_size *= 1024;

  size_t headroom_delta = ~0;
  if (min_headroom != 0) {
    const bool privileged = true;
    const size_t free = memfaultd_get_free_space(handle->core_dir, privileged);
    if (free <= min_headroom) {
      return 0;
    }
    headroom_delta = free - min_headroom;
  }

  size_t usage_delta = ~0;
  if (max_usage != 0) {
    const size_t used = memfaultd_get_folder_size(handle->core_dir);
    if (used >= max_usage) {
      return 0;
    }
    usage_delta = max_usage - used;
  }

  return MEMFAULT_MIN(MEMFAULT_MIN(headroom_delta, usage_delta), max_size);
}

static bool prv_msg_handler(sMemfaultdPlugin *handle, struct msghdr *msg, size_t received_size) {
  int ret = EXIT_FAILURE;
  char *buf = msg->msg_iov[0].iov_base;
  char *outfile = NULL;
  sMemfaultdTxData *data = NULL;
  int file_stream = -1;

  // Get file stream descriptor from message
  for (struct cmsghdr *c = CMSG_FIRSTHDR(msg); c; c = CMSG_NXTHDR(msg, c)) {
    if (c->cmsg_level == SOL_SOCKET && c->cmsg_type == SCM_RIGHTS) {
      file_stream = *(int *)CMSG_DATA(c);
      break;
    }
  }
  if (file_stream == -1) {
    fprintf(stderr, "coredump:: Failed to find stream file descriptor\n");
    goto cleanup;
  }

  size_t offset = 0;
  const char *match = buf;
  offset += strnlen(match, received_size - offset) + 1;

  if (offset >= received_size) {
    fprintf(stderr, "coredump:: Match subtype missing\n");
    goto cleanup;
  }
  const char *match_subtype = &buf[offset];
  offset += strnlen(match_subtype, received_size - offset) + 1;

  if (strcmp("ELF", match_subtype) != 0) {
    fprintf(stderr, "coredump:: Unrecognised subtype\n");
  }

  if (offset >= received_size) {
    fprintf(stderr, "coredump:: PID missing\n");
    goto cleanup;
  }

  char *endptr;
  const pid_t pid = strtol(&buf[offset], &endptr, 10);
  if (&buf[offset] == endptr || *endptr != '\0') {
    fprintf(stderr, "coredump:: Invalid PID in message\n");
    goto cleanup;
  }

  prv_log_coredump_request(pid);

  if (!handle->enable_data_collection) {
    fprintf(stderr, "coredump:: Data collection disabled, not processing corefile\n");
    ret = EXIT_SUCCESS;
    goto cleanup;
  }

  if (!memfaultd_rate_limiter_check_event(handle->rate_limiter)) {
    ret = EXIT_SUCCESS;
    goto cleanup;
  }

  const size_t max_size = prv_check_for_available_space(handle);
  if (max_size == 0) {
    fprintf(stderr, "coredump:: Not processing corefile, disk usage limits exceeded\n");
    goto cleanup;
  }

  // Write corefile to fs
  const char *extension = handle->gzip_enabled ? ".gz" : "";
  if ((outfile = prv_generate_filename(handle, "corefile-", extension)) == NULL) {
    goto cleanup;
  }

  fprintf(stderr, "coredump:: writing coredump with max size: %lu\n", (long unsigned int)max_size);
  if (!prv_transform_coredump_from_fd_to_file(handle, outfile, file_stream, pid, max_size)) {
    if (unlink(outfile) == -1 && errno != ENOENT) {
      fprintf(stderr, "coredump:: Failed to remove core file '%s' after failure : %s\n", outfile,
              strerror(errno));
    }
    goto cleanup;
  }

  // Add outfile to queue for transmission
  uint32_t payload_size;
  data = prv_build_queue_entry(handle->gzip_enabled, outfile, &payload_size);
  if (!data) {
    fprintf(stderr, "coredump:: Failed to build queue entry\n");
    goto cleanup;
  }

  if (!memfaultd_txdata(handle->memfaultd, data, payload_size)) {
    fprintf(stderr, "coredump:: Failed to queue corefile\n");
    goto cleanup;
  }

  fprintf(stderr, "coredump:: enqueued corefile for PID %d\n", pid);

  ret = EXIT_SUCCESS;

cleanup:
  free(data);
  if (file_stream != -1) {
    close(file_stream);
  }

  if (ret != EXIT_SUCCESS) {
    if (outfile && unlink(outfile) == -1 && errno != ENOENT) {
      fprintf(stderr, "Failed to remove core file '%s' after failure : %s\n", outfile,
              strerror(errno));
    }
  }
  free(outfile);

  return (ret == EXIT_SUCCESS);
}

/**
 * @brief Destroys ipc plugin
 *
 * @param memfaultd ipc plugin handle
 */
static void prv_destroy(sMemfaultdPlugin *handle) {
  if (handle) {
    memfaultd_rate_limiter_destroy(handle->rate_limiter);
    free(handle->core_dir);
    free(handle);
  }
}

static sMemfaultdPluginCallbackFns s_fns = {
  .plugin_destroy = prv_destroy,
  .plugin_ipc_msg_handler = prv_msg_handler,
};

/**
 * @brief Initialises ipc plugin
 *
 * @param memfaultd Main memfaultd handle
 * @return callbackFunctions_t Plugin function table
 */
bool memfaultd_coredump_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns) {
  sMemfaultdPlugin *handle = NULL;
  int fd = -1;
  if ((handle = calloc(sizeof(sMemfaultdPlugin), 1)) == NULL) {
    fprintf(stderr, "coredump:: Failed to create handler context\n");
    goto cleanup;
  }

  handle->memfaultd = memfaultd;
  *fns = &s_fns;
  (*fns)->handle = handle;

  if (!memfaultd_get_boolean(handle->memfaultd, "", "enable_data_collection",
                             &handle->enable_data_collection) ||
      !handle->enable_data_collection) {
    //! Even though comms are disabled, we still want to log any crashes which have happened
    fprintf(stderr, "coredump:: Data collection is off, plugin disabled.\n");
  }

  if (!(handle->core_dir = prv_create_dir(handle, "core"))) {
    fprintf(stderr, "coredump:: Unable to create core directory.\n");
    goto cleanup;
  }

  // Write core_patten to kernel
  if ((fd = open(CORE_PATTERN_PATH, O_WRONLY, 0)) == -1) {
    fprintf(stderr, "coredump:: Failed to open kernel core pattern file : %s\n", strerror(errno));
    goto cleanup;
  }
  if (write(fd, CORE_PATTERN, strlen(CORE_PATTERN)) == -1) {
    fprintf(stderr, "coredump:: Failed to write kernel core pattern : %s\n", strerror(errno));
    goto cleanup;
  }
  close(fd);

  handle->rate_limiter = coredump_create_rate_limiter(handle->memfaultd);

  const char *compression = COMPRESSION_DEFAULT;
  memfaultd_get_string(handle->memfaultd, "coredump_plugin", "compression", &compression);
  if (strcmp(compression, "gzip") == 0) {
    handle->gzip_enabled = true;
  } else if (strcmp(compression, "none") == 0) {
    handle->gzip_enabled = false;
  } else {
    fprintf(stderr,
            "coredump:: Invalid configuration: coredump_plugin.compression value '%s' - Use "
            "'none' or 'gzip'.\n",
            compression);
    handle->gzip_enabled = true;
  }

  return true;

cleanup:
  memfaultd_rate_limiter_destroy(handle->rate_limiter);
  free(handle->core_dir);
  free(handle);
  if (fd != -1) {
    close(fd);
  }
  return false;
}
