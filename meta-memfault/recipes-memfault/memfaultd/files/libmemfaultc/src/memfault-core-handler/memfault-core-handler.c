//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfault-core-handler program that accepts coredumps from the Linux kernel.

#include "memfault-core-handler.h"

#include <getopt.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <syslog.h>
#include <uuid/uuid.h>

#include "core_elf_process_fd.h"
#include "coredump_ratelimiter.h"
#include "memfault/util/config.h"
#include "memfault/util/device_settings.h"
#include "memfault/util/disk.h"
#include "memfault/util/logging.h"
#include "memfault/util/rate_limiter.h"
#include "memfault/util/string.h"

#define COMPRESSION_DEFAULT "gzip"

#ifndef MEMFAULT_CORE_HANDLER_LOG_LEVEL
  #define MEMFAULT_CORE_HANDLER_LOG_LEVEL (LOG_INFO)
#endif

static char *prv_create_output_dir(sMemfaultdConfig *config) {
  char *path = memfaultd_config_generate_tmp_filename(config, "core");
  if (path == NULL) {
    return path;
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

static size_t prv_calculate_available_space(sMemfaultdConfig *config, const char *core_dir) {
  sMemfaultStorageQuota quota = {0};

  memfaultd_config_get_integer(config, "", "tmp_dir_min_headroom_kib", (int *)&quota.min_headroom);
  memfaultd_config_get_integer(config, "", "tmp_dir_max_usage_kib", (int *)&quota.max_usage);
  memfaultd_config_get_integer(config, "coredump_plugin", "coredump_max_size_kib",
                               (int *)&quota.max_size);

  quota.min_headroom *= 1024;
  quota.max_usage *= 1024;
  quota.max_size *= 1024;

  return memfaultd_calculate_available_space(core_dir, &quota);
}

static char *prv_generate_filename(const char *output_dir, const char *prefix,
                                   const char *extension) {
  char *filename = NULL;

  uuid_t uuid;
  char uuid_str[37];
  uuid_generate(uuid);
  uuid_unparse_lower(uuid, uuid_str);

  char *fmt = "%s/%s%s%s";
  if (memfault_asprintf(&filename, fmt, output_dir, prefix, uuid_str, extension) == -1) {
    fprintf(stderr, "coredump:: Failed to create filename buffer\n");
    goto cleanup;
  }

  return filename;

cleanup:
  free(filename);
  return NULL;
}

static bool prv_gzip_enabled(sMemfaultdConfig *config) {
  const char *compression = COMPRESSION_DEFAULT;
  memfaultd_config_get_string(config, "coredump_plugin", "compression", &compression);
  return (strcmp(compression, "gzip") == 0);
}

int memfault_core_handler_main(int argc, char *argv[]) {
#if __linux__
  //! Disable coredumping of this process
  prctl(PR_SET_DUMPABLE, 0, 0, 0);
#endif

  memfaultd_log_configure(kMemfaultdLogLevel_Debug, kMemfaultdLogDestination_SystemdJournal);

  memfaultd_log(kMemfaultdLogLevel_Info, "Starting memfault-core-handler");

  char *config_file = NULL;
  char *output_dir = NULL;
  char *output_file = NULL;
  sMemfaultdConfig *config = NULL;
  sMemfaultdRateLimiter *rate_limiter = NULL;
  sMemfaultdDeviceSettings *device_settings = NULL;
  eMemfaultCoreHandlerStatus status_code = kMemfaultCoreHandlerStatus_Ok;

  int opt;
  while ((opt = getopt(argc, argv, "+c:")) != -1) {
    switch (opt) {
      case 'c':
        config_file = optarg;
        break;
      default:
        status_code = kMemfaultCoreHandlerStatus_InvalidArguments;
        goto cleanup;
    }
  }

  if (config_file == NULL) {
    status_code = kMemfaultCoreHandlerStatus_InvalidArguments;
    goto cleanup;
  }
  if (optind >= argc) {
    status_code = kMemfaultCoreHandlerStatus_InvalidArguments;
    goto cleanup;
  }
  const pid_t pid = (pid_t)strtol(argv[optind], NULL, 10);

  config = memfaultd_config_init(config_file);
  if (!config) {
    status_code = kMemfaultCoreHandlerStatus_InvalidConfiguration;
    memfaultd_log(kMemfaultdLogLevel_Error, "Invalid configuration file");
    goto cleanup;
  }

  bool allowed;
  if (!memfaultd_config_get_boolean(config, NULL, CONFIG_KEY_DATA_COLLECTION, &allowed) ||
      !allowed) {
    memfaultd_log(kMemfaultdLogLevel_Error, "Data collection disabled, not processing corefile");
    goto cleanup;
  }

  // Check the rate limiter up front - create_rate_limiter returns NULL when disabled
  rate_limiter = coredump_create_rate_limiter(config);
  if (!memfaultd_rate_limiter_check_event(rate_limiter)) {
    memfaultd_log(kMemfaultdLogLevel_Info, "Limit reached, not processing corefile");
    goto cleanup;
  }

  if ((device_settings = memfaultd_device_settings_init()) == NULL) {
    status_code = kMemfaultCoreHandlerStatus_DeviceSettingsFailure;
    memfaultd_log(kMemfaultdLogLevel_Error, "Failed to get device settings");
    goto cleanup;
  }

  if ((output_dir = prv_create_output_dir(config)) == NULL) {
    memfaultd_log(kMemfaultdLogLevel_Error, "Failed to generate core directory");
    status_code = kMemfaultCoreHandlerStatus_OOM;
    goto cleanup;
  }

  const size_t max_size = prv_calculate_available_space(config, output_dir);
  if (max_size == 0) {
    memfaultd_log(kMemfaultdLogLevel_Info, "Not processing corefile, disk usage limits exceeded");
    status_code = kMemfaultCoreHandlerStatus_DiskQuotaExceeded;
    goto cleanup;
  }

  const bool gzip_enabled = prv_gzip_enabled(config);
  if ((output_file = prv_generate_filename(output_dir, "corefile-", gzip_enabled ? ".gz" : "")) ==
      NULL) {
    memfaultd_log(kMemfaultdLogLevel_Error, "Failed to generate filename");
    status_code = kMemfaultCoreHandlerStatus_OOM;
    goto cleanup;
  }

  sMemfaultProcessCoredumpCtx ctx = {
    .input_fd = STDIN_FILENO,
    .pid = pid,
    .device_settings = device_settings,
    .software_type = NULL,
    .software_version = NULL,
    .output_file = output_file,
    .max_size = max_size,
    .gzip_enabled = gzip_enabled,
  };

  if (!memfaultd_config_get_string(config, "", "software_type", &ctx.software_type) ||
      strlen(ctx.software_type) == 0) {
    memfaultd_log(kMemfaultdLogLevel_Error, "Failed to get software_type");
    status_code = kMemfaultCoreHandlerStatus_InvalidConfiguration;
    goto cleanup;
  }

  if (!memfaultd_config_get_string(config, "", "software_version", &ctx.software_version) ||
      strlen(ctx.software_version) == 0) {
    memfaultd_log(kMemfaultdLogLevel_Error, "Failed to get software_version");
    status_code = kMemfaultCoreHandlerStatus_InvalidConfiguration;
    goto cleanup;
  }

  if (core_elf_process_fd(&ctx)) {
    memfaultd_log(kMemfaultdLogLevel_Info, "Successfully captured coredump");
  } else {
    status_code = EXIT_FAILURE;
  }

cleanup:
  free(output_dir);
  free(output_file);
  memfaultd_rate_limiter_destroy(rate_limiter);
  memfaultd_device_settings_destroy(device_settings);
  memfaultd_config_destroy(config);

  return status_code;
}
