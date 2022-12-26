//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfault collectd plugin implementation

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <systemd/sd-bus.h>
#include <unistd.h>

#include "memfault/core/math.h"
#include "memfault/util/string.h"
#include "memfault/util/systemd.h"
#include "memfaultd.h"

#define DEFAULT_HEADER_INCLUDE_OUTPUT_FILE "/tmp/collectd-header-include.conf"
#define DEFAULT_FOOTER_INCLUDE_OUTPUT_FILE "/tmp/collectd-footer-include.conf"
#define DEFAULT_INTERVAL_SECS 3600

#define COLLECTD_PATH "/api/v0/collectd"
#define MEMFAULT_HEADER "Memfault-Project-Key"

struct MemfaultdPlugin {
  sMemfaultd *memfaultd;
  bool was_enabled;
  const char *header_include_output_file;
  const char *footer_include_output_file;
};

/**
 * @brief Generate new collectd-header-include.conf file from config
 *
 * @param handle collectd plugin handle
 * @param override_interval if greater than 0, will override the interval in configuration
 * @return true Successfully generated new config
 * @return false Failed to generate
 */
bool prv_generate_header_include(sMemfaultdPlugin *handle, int override_interval) {
  bool result = true;
  FILE *fd = NULL;

  int interval_seconds = DEFAULT_INTERVAL_SECS;
  memfaultd_get_integer(handle->memfaultd, "collectd_plugin", "interval_seconds",
                        &interval_seconds);

  if (override_interval > 0) {
    interval_seconds = override_interval;
  }

  fd = fopen(handle->header_include_output_file, "w+");
  if (!fd) {
    fprintf(stderr, "collectd:: Failed to open output file: %s\n",
            handle->header_include_output_file);
    result = false;
    goto cleanup;
  }

  char *globals_fmt = "Interval %d\n\n";
  if (fprintf(fd, globals_fmt, interval_seconds) == -1) {
    fprintf(stderr, "collectd:: Failed to write Interval\n");
    result = false;
    goto cleanup;
  }

cleanup:
  if (fd != NULL) {
    fclose(fd);
  }
  return result;
}

bool prv_generate_write_http(sMemfaultdPlugin *handle, FILE *fd) {
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(handle->memfaultd);

  const char *base_url, *software_type, *software_version, *project_key;
  if (!memfaultd_get_string(handle->memfaultd, "", "base_url", &base_url)) {
    return false;
  }
  if (!memfaultd_get_string(handle->memfaultd, "", "software_type", &software_type)) {
    return false;
  }
  if (!memfaultd_get_string(handle->memfaultd, "", "software_version", &software_version)) {
    return false;
  }
  if (!memfaultd_get_string(handle->memfaultd, "", "project_key", &project_key)) {
    return false;
  }
  int interval_seconds = 0;
  memfaultd_get_integer(handle->memfaultd, "collectd_plugin", "interval_seconds",
                        &interval_seconds);

  bool result = true;
  char *url = NULL;
  char *add_header = NULL;

  int buffer_size = 64;
  memfaultd_get_integer(handle->memfaultd, "collectd_plugin", "write_http_buffer_size_kib",
                        &buffer_size);
  buffer_size *= 1024;

  // Future: read from remote Memfault device config.
  bool store_rates = true;  // Otherwise most metrics are reported as cumulative values.
  int low_speed_limit = 0;
  int timeout = 0;

  char *url_fmt = "%s%s/%s/%s/%s/%s";
  if (memfault_asprintf(&url, url_fmt, base_url, COLLECTD_PATH, settings->device_id,
                        settings->hardware_version, software_type, software_version) == -1) {
    fprintf(stderr, "collectd:: Failed to create url buffer\n");
    result = false;
    goto cleanup;
  }

  char *add_header_fmt = "%s: %s";
  if (memfault_asprintf(&add_header, add_header_fmt, MEMFAULT_HEADER, project_key) == -1) {
    fprintf(stderr, "collectd:: Failed to create additional headers buffer\n");
    result = false;
    goto cleanup;
  }

  char *write_http_fmt = "<LoadPlugin write_http>\n"
                         "  FlushInterval %d\n"
                         "</LoadPlugin>\n\n"
                         "<Plugin write_http>\n"
                         "  <Node \"memfault\">\n"
                         "    URL \"%s\"\n"
                         "    VerifyPeer true\n"
                         "    VerifyHost true\n"
                         "    Header \"%s\"\n"
                         "    Format \"JSON\"\n"
                         "    Metrics true\n"
                         "    Notifications false\n"
                         "    StoreRates %s\n"
                         "    BufferSize %d\n"
                         "    LowSpeedLimit %d\n"
                         "    Timeout %d\n"
                         "  </Node>\n"
                         "</Plugin>\n\n";
  if (fprintf(fd, write_http_fmt, interval_seconds, url, add_header, store_rates ? "true" : "false",
              buffer_size, low_speed_limit, timeout) == -1) {
    fprintf(stderr, "collectd:: Failed to write write_http statement\n");
    result = false;
    goto cleanup;
  }

cleanup:
  free(url);
  free(add_header);

  return result;
}

bool prv_generate_chain(sMemfaultdPlugin *handle, FILE *fd) {
  // TODO: Add filtering once structure has been agreed on
  bool result = true;
  const char *non_memfault_chain;
  char *target = NULL;

  if (!memfaultd_get_string(handle->memfaultd, "collectd_plugin", "non_memfaultd_chain",
                            &non_memfault_chain) ||
      strlen(non_memfault_chain) == 0) {
    target = strdup("    Target \"stop\"\n");
  } else {
    char *target_fmt = "    <Target \"jump\">\n"
                       "      Chain \"%s\"\n"
                       "    </Target>";
    if (memfault_asprintf(&target, target_fmt, non_memfault_chain) == -1) {
      fprintf(stderr, "collectd:: Failed to create target buffer\n");
      result = false;
      goto cleanup;
    }
  }

  char *chain_fmt = "LoadPlugin match_regex\n"
                    "PostCacheChain \"MemfaultdGeneratedPostCacheChain\"\n"
                    "<Chain \"MemfaultdGeneratedPostCacheChain\">\n"
                    "  <Rule \"ignore_memory_metrics\">\n"
                    "    <Match \"regex\">\n"
                    "      Type \"^memory$\"\n"
                    "      TypeInstance \"^(buffered|cached|slab_recl|slab_unrecl)$\"\n"
                    "    </Match>\n"
                    "%s"
                    "  </Rule>\n"
                    "  Target \"write\"\n"
                    "</Chain>\n\n";
  if (fprintf(fd, chain_fmt, target) == -1) {
    fprintf(stderr, "collectd:: Failed to create chain buffer\n");
    result = false;
    goto cleanup;
  }

cleanup:
  free(target);

  return result;
}

/**
 * @brief Generate new collectd-postamble.conf file from config
 *
 * @param handle collectd plugin handle
 * @return true Successfully generated new config
 * @return false Failed to generate
 */
static bool prv_generate_footer_include(sMemfaultdPlugin *handle) {
  bool result = true;

  FILE *fd = fopen(handle->footer_include_output_file, "w+");
  if (!fd) {
    fprintf(stderr, "collectd:: Failed to open output file: %s\n",
            handle->footer_include_output_file);
    result = false;
    goto cleanup;
  }

  if (!prv_generate_write_http(handle, fd)) {
    result = false;
    goto cleanup;
  }

  if (!prv_generate_chain(handle, fd)) {
    result = false;
    goto cleanup;
  }

cleanup:
  if (fd != NULL) {
    fclose(fd);
  }
  return result;
}

/**
 * @brief Destroys collectd plugin
 *
 * @param memfaultd collectd plugin handle
 */
static void prv_destroy(sMemfaultdPlugin *handle) {
  if (handle) {
    free(handle);
  }
}

/**
 * @brief Gets the size of the file for the given file path.
 * @param file_path The file path.
 * @return Size of the file or -errno in case of an error.
 */
static ssize_t prv_get_file_size(const char *file_path) {
  if (access(file_path, F_OK) != 0) {
    return -errno;
  }
  struct stat st;
  stat(file_path, &st);
  return st.st_size;
}

/**
 * @brief Empties the given file for the given file path.
 * @param file_path The file path.
 */
static bool prv_write_empty_file(const char *file_path) {
  FILE *fd = fopen(file_path, "w+");
  if (!fd) {
    fprintf(stderr, "collectd:: Failed to open output file: %s\n", file_path);
    return false;
  }
  fclose(fd);
  return true;
}

/**
 * Clears the config files, but only if they are not already cleared.
 * @param handle collectd plugin handle
 * @return True if one or more files were cleared, or false if all files had already been cleared,
 * or the files did not exist before.
 */
static bool prv_clear_config_files_if_not_already_cleared(sMemfaultdPlugin *handle) {
  bool did_clear = false;
  const char *output_files[] = {
    handle->header_include_output_file,
    handle->footer_include_output_file,
  };
  for (size_t i = 0; i < MEMFAULT_ARRAY_SIZE(output_files); ++i) {
    const ssize_t rv = prv_get_file_size(output_files[i]);
    const size_t should_clear = rv > 0;
    if (should_clear || rv == -ENOENT /* file does not exist yet */) {
      prv_write_empty_file(output_files[i]);
    }
    did_clear |= should_clear != 0;
  }
  return did_clear;
}

/**
 * @brief Reload collectd plugin
 *
 * @param handle collectd plugin handle
 * @return true Successfully reloaded collectd plugin
 * @return false Failed to reload plugin
 */
static bool prv_reload(sMemfaultdPlugin *handle) {
  if (!handle) {
    return false;
  }

  bool enabled;
  if (!memfaultd_get_boolean(handle->memfaultd, "", "enable_data_collection", &enabled) ||
      !enabled) {
    // Data collection is disabled
    fprintf(stderr, "collectd:: Data collection is off, plugin disabled.\n");

    const bool needs_restart = prv_clear_config_files_if_not_already_cleared(handle);

    if (handle->was_enabled || needs_restart) {
      // Data collection only just disabled

      if (!memfaultd_restart_service_if_running("collectd.service")) {
        fprintf(stderr, "collectd:: Failed to restart collectd\n");
        return false;
      }

      handle->was_enabled = false;
    }
  } else {
    // Data collection enabled
    if (!prv_generate_header_include(handle, 0)) {
      fprintf(stderr, "collectd:: Failed to generate updated header config file\n");
      return false;
    }
    if (!prv_generate_footer_include(handle)) {
      fprintf(stderr, "collectd:: Failed to generate updated footer config file\n");
      return false;
    }

    if (!memfaultd_restart_service_if_running("collectd.service")) {
      fprintf(stderr, "collectd:: Failed to restart collectd\n");
      return false;
    }

    handle->was_enabled = true;
  }
  return true;
}

static bool prv_request_metrics(sMemfaultdPlugin *handle) {
  if (handle->was_enabled) {
    // Restarting collectd forces a new measurement of all monitored values.
    if (!memfaultd_restart_service_if_running("collectd.service")) {
      fprintf(stderr, "collectd:: Failed to restart collectd\n");
      return false;
    }

    // Make sure we give collectd time to take the measurement
    sleep(1);

    // And now force collectd to flush the measurements in cache.
    fprintf(stderr, "collectd:: Requesting metrics from collectd now.\n");
    memfaultd_kill_service("collectd.service", SIGUSR1);
  } else {
    fprintf(stderr, "collected:: Metrics are not enabled.\n");
  }
  return true;
}

static bool prv_ipc_handler(sMemfaultdPlugin *handle, struct msghdr *msg, size_t received_size) {
  // Any IPC message will cause us to request metrics.
  return prv_request_metrics(handle);
}

static sMemfaultdPluginCallbackFns s_fns = {.plugin_destroy = prv_destroy,
                                            .plugin_reload = prv_reload,
                                            .plugin_ipc_msg_handler = prv_ipc_handler};

/**
 * @brief Initialises collectd plugin
 *
 * @param memfaultd Main memfaultd handle
 * @return callbackFunctions_t Plugin function table
 */
bool memfaultd_collectd_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns) {
  sMemfaultdPlugin *handle = calloc(sizeof(sMemfaultdPlugin), 1);
  if (!handle) {
    fprintf(stderr, "collectd:: Failed to allocate plugin handle\n");
    return false;
  }

  handle->memfaultd = memfaultd;
  *fns = &s_fns;
  (*fns)->handle = handle;

  memfaultd_get_boolean(handle->memfaultd, "", "enable_data_collection", &handle->was_enabled);

  if (!memfaultd_get_string(handle->memfaultd, "collectd_plugin", "header_include_output_file",
                            &handle->header_include_output_file)) {
    handle->header_include_output_file = DEFAULT_HEADER_INCLUDE_OUTPUT_FILE;
  }

  if (!memfaultd_get_string(handle->memfaultd, "collectd_plugin", "footer_include_output_file",
                            &handle->footer_include_output_file)) {
    handle->footer_include_output_file = DEFAULT_FOOTER_INCLUDE_OUTPUT_FILE;
  }

  // Ignore failures after this point as we still want setting changes to attempt to reload the
  // config

  prv_reload(handle);

  return true;
}
