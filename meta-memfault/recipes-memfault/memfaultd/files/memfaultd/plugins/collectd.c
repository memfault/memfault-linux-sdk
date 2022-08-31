//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfault collectd plugin implementation

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <systemd/sd-bus.h>
#include <unistd.h>

#include "memfaultd.h"
#include "memfaultd_utils.h"

#define DEFAULT_OUTPUT_FILE "/tmp/collectd.conf"

#define COLLECTD_PATH "/api/v0/collectd"
#define MEMFAULT_HEADER "Memfault-Project-Key"

struct MemfaultdPlugin {
  sMemfaultd *memfaultd;
  bool was_enabled;
};

char *prv_generate_globals(sMemfaultdPlugin *handle) {
  char *globals = NULL;
  int interval_seconds = 0;
  memfaultd_get_integer(handle->memfaultd, "collectd", "interval_seconds", &interval_seconds);

  int len;
  char *globals_fmt = "Interval %d\n\n";
  len = snprintf(NULL, 0, globals_fmt, interval_seconds);
  if (!(globals = malloc(len + 1))) {
    fprintf(stderr, "collectd:: Failed to create write_http buffer\n");
    goto cleanup;
  }
  sprintf(globals, globals_fmt, interval_seconds);

cleanup:
  return globals;
}

char *prv_generate_write_http(sMemfaultdPlugin *handle) {
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
  memfaultd_get_integer(handle->memfaultd, "collectd", "interval_seconds", &interval_seconds);

  char *url = NULL;
  char *add_header = NULL;
  char *write_http = NULL;

  int buffer_size = 64;
  memfaultd_get_integer(handle->memfaultd, "collectd", "write_http_buffer_size_kib", &buffer_size);
  buffer_size *= 1024;

  // Future: read from remote Memfault device config.
  bool store_rates = true;  // Otherwise most metrics are reported as cumulative values.
  int low_speed_limit = 0;
  int timeout = 0;

  int len;
  char *url_fmt = "%s%s/%s/%s/%s/%s";
  len = snprintf(NULL, 0, url_fmt, base_url, COLLECTD_PATH, settings->device_id,
                 settings->hardware_version, software_type, software_version);
  if (!(url = malloc(len + 1))) {
    fprintf(stderr, "collectd:: Failed to create url buffer\n");
    goto cleanup;
  }
  sprintf(url, url_fmt, base_url, COLLECTD_PATH, settings->device_id, settings->hardware_version,
          software_type, software_version);

  char *add_header_fmt = "%s: %s";
  len = snprintf(NULL, 0, add_header_fmt, MEMFAULT_HEADER, project_key);
  if (!(add_header = malloc(len + 1))) {
    fprintf(stderr, "collectd:: Failed to create additional headers buffer\n");
    goto cleanup;
  }
  sprintf(add_header, add_header_fmt, MEMFAULT_HEADER, project_key);

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
  len = snprintf(NULL, 0, write_http_fmt, interval_seconds, url, add_header,
                 store_rates ? "true" : "false", buffer_size, low_speed_limit, timeout);
  if (!(write_http = malloc(len + 1))) {
    fprintf(stderr, "collectd:: Failed to create write_http buffer\n");
    goto cleanup;
  }
  sprintf(write_http, write_http_fmt, interval_seconds, url, add_header,
          store_rates ? "true" : "false", buffer_size, low_speed_limit, timeout);

cleanup:
  free(url);
  free(add_header);

  return write_http;
}

char *prv_generate_chain(sMemfaultdPlugin *handle) {
  // TODO: Add filtering once structure has been agreed on

  const char *non_memfault_chain;
  char *target = NULL;
  char *chain = NULL;
  int len;

  if (!memfaultd_get_string(handle->memfaultd, "collectd", "non_memfaultd_chain",
                            &non_memfault_chain) ||
      strlen(non_memfault_chain) == 0) {
    target = strdup("    Target \"stop\"\n");
  } else {
    char *target_fmt = "    <Target \"jump\">\n"
                       "      Chain \"%s\"\n"
                       "    </Target>";
    len = snprintf(NULL, 0, target_fmt, non_memfault_chain);
    if (!(target = malloc(len + 1))) {
      fprintf(stderr, "collectd:: Failed to create target buffer\n");
      goto cleanup;
    }
    sprintf(target, target_fmt, non_memfault_chain);
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
  len = snprintf(NULL, 0, chain_fmt, target);
  if (!(chain = malloc(len + 1))) {
    fprintf(stderr, "collectd:: Failed to create chain buffer\n");
    goto cleanup;
  }
  sprintf(chain, chain_fmt, target, target);

cleanup:
  free(target);

  return chain;
}

/**
 * @brief Generate new collectd.conf file from config
 *
 * @param handle collectd plugin handle
 * @return true Successfully generated new config
 * @return false Failed to generate
 */
static bool prv_generate_config(sMemfaultdPlugin *handle) {
  char *globals = NULL;
  char *write_http = NULL;
  char *chain = NULL;
  bool result = true;

  globals = prv_generate_globals(handle);
  if (!globals) {
    result = false;
    goto cleanup;
  }

  write_http = prv_generate_write_http(handle);
  if (!write_http) {
    result = false;
    goto cleanup;
  }

  chain = prv_generate_chain(handle);
  if (!chain) {
    result = false;
    goto cleanup;
  }

  const char *output_file;
  if (!memfaultd_get_string(handle->memfaultd, "collectd_plugin", "output_file", &output_file)) {
    output_file = DEFAULT_OUTPUT_FILE;
  }

  FILE *fd = fopen(output_file, "w+");
  if (!fd) {
    fprintf(stderr, "collectd:: Failed to open output file\n");
    result = false;
    goto cleanup;
  }

  fputs(globals, fd);
  fputs(write_http, fd);
  fputs(chain, fd);

  fclose(fd);

cleanup:
  free(write_http);
  free(chain);
  free(globals);

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

    const char *output_file;
    if (!memfaultd_get_string(handle->memfaultd, "collectd_plugin", "output_file", &output_file)) {
      output_file = DEFAULT_OUTPUT_FILE;
    }

    int old_file_size = 0;
    if (access(output_file, F_OK) == 0) {
      struct stat st;

      stat(output_file, &st);
      old_file_size = st.st_size;
    }

    // Create empty config fragment
    FILE *fd = fopen(output_file, "w+");
    if (!fd) {
      return false;
    } else {
      fclose(fd);
    }

    if (handle->was_enabled || old_file_size != 0) {
      // Data collection only just disabled

      if (!memfaultd_utils_restart_service_if_running("collectd", "collectd.service")) {
        fprintf(stderr, "collectd:: Failed to restart collectd\n");
        return false;
      }

      handle->was_enabled = false;
    }
  } else {
    // Data collection enabled
    if (!prv_generate_config(handle)) {
      fprintf(stderr, "collectd:: Failed to generate updated config file\n");
      return false;
    }

    if (!memfaultd_utils_restart_service_if_running("collectd", "collectd.service")) {
      fprintf(stderr, "collectd:: Failed to restart collectd\n");
      return false;
    }

    handle->was_enabled = true;
  }
  return true;
}

static sMemfaultdPluginCallbackFns s_fns = {
  .plugin_destroy = prv_destroy,
  .plugin_reload = prv_reload,
};

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

  // Ignore failures after this point as we still want setting changes to attempt to reload the
  // config

  prv_reload(handle);

  return true;
}
