//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! reboot reason plugin implementation

// clang-format off
// libuboot.h requires size_t from stddef.h
#include <stddef.h>
#include <libuboot.h>
// clang-format on

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <systemd/sd-bus.h>
#include <unistd.h>

#include "memfaultd.h"

#define PSTORE_FILE "/sys/fs/pstore/dmesg-ramoops-0"
#define FWENV_CONFIG_FILE "/etc/fw_env.config"

#define REBOOTREASON_UNKNOWN 0x0000
#define REBOOTREASON_USERRESET 0x0002
#define REBOOTREASON_SOFTWAREUPDATE 0x0003
#define REBOOTREASON_LOWPOWER 0x0004
#define REBOOTREASON_WATCHDOG 0x8002
#define REBOOTREASON_KERNELPANIC 0x8003

struct MemfaultdPlugin {
  sMemfaultd *memfaultd;
};

/**
 * @brief Builds event JSON object for posting to events API
 *
 * @param handle reboot plugin handle
 * @param val reboot reason number to encode
 * @param userinfo optional userinfo string
 * @param payload_size size of the payload in the returned data object
 * @return sMemfaultdTxData* Tx data with the reboot event
 */
static sMemfaultdTxData *prv_reboot_build_event(sMemfaultdPlugin *handle, const int val,
                                                const char *userinfo, uint32_t *payload_size) {
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(handle->memfaultd);

  const char *software_type = "";
  const char *software_version = "";
  memfaultd_get_string(handle->memfaultd, "", "software_type", &software_type);
  memfaultd_get_string(handle->memfaultd, "", "software_version", &software_version);

  const size_t max_event_size = 1024;
  sMemfaultdTxData *data = malloc(sizeof(sMemfaultdTxData) + max_event_size);
  if (data == NULL) {
    fprintf(stderr, "reboot:: Failed to build event structure, out of memory\n");
    return NULL;
  }
  data->type = kMemfaultdTxDataType_RebootEvent;

  char *str = (char *)data->payload;
  const int ret = snprintf(str, max_event_size,
                           "["
                           "{"
                           "\"type\": \"trace\","
                           "\"software_type\": \"%s\","
                           "\"software_version\": \"%s\","
                           "\"device_serial\": \"%s\","
                           "\"hardware_version\": \"%s\","
                           "\"sdk_version\": \"0.5.0\","
                           "\"event_info\": {"
                           "\"reason\": %d"
                           "},"
                           "\"user_info\": {%s}"
                           "}"
                           "]",
                           software_type, software_version, settings->device_id,
                           settings->hardware_version, val, userinfo ? userinfo : "");
  if (ret >= max_event_size || ret < 0) {
    fprintf(stderr, "reboot:: Failed to build event structure %d\n", ret);
    free(data);
    return NULL;
  }

  *payload_size = ret + 1 /* NUL terminator */;
  return data;
}

/**
 * @brief Writes given reboot reason to file
 *
 * @param handle reboot plugin handle
 * @param reboot_reason Reason to store
 */
static void prv_reboot_write_reboot_reason(sMemfaultdPlugin *handle, int reboot_reason) {
  char *file = memfaultd_generate_rw_filename(handle->memfaultd, "lastrebootreason");
  if (!file) {
    fprintf(stderr, "reboot:: Failed to get reboot reason file\n");
    return;
  }

  FILE *fd = fopen(file, "w+");
  free(file);
  if (!fd) {
    fprintf(stderr, "reboot:: Failed to open reboot reason file\n");
    return;
  }

  fprintf(fd, "%d", reboot_reason);

  fclose(fd);
}

/**
 * @brief Reads reboot reason from file and then deletes it
 *
 * @param handle reboot plugin handle
 * @return int reboot_reason read from file
 */
static int prv_reboot_read_and_clear_reboot_reason(sMemfaultdPlugin *handle) {
  char *file = memfaultd_generate_rw_filename(handle->memfaultd, "lastrebootreason");
  if (!file) {
    fprintf(stderr, "reboot:: Failed to get reboot reason file\n");
    return REBOOTREASON_UNKNOWN;
  }

  FILE *fd = fopen(file, "r");
  if (!fd) {
    free(file);
    return REBOOTREASON_UNKNOWN;
  }

  int reboot_reason;
  if (fscanf(fd, "%d", &reboot_reason) != 1) {
    reboot_reason = REBOOTREASON_UNKNOWN;
  }

  fclose(fd);

  unlink(file);

  free(file);

  return reboot_reason;
}

/**
 * @brief Checks if the current systemd state matches the requested state
 *
 * @param handle reboot plugin handle
 * @param state State to validate against
 * @return true Machine is in requested state
 * @return false Machine is not
 */
static bool prv_reboot_is_systemd_state(sMemfaultdPlugin *handle, const char *state) {
  sd_bus *bus;
  sd_bus_error error = SD_BUS_ERROR_NULL;
  char *cur_state;

  const char *service = "org.freedesktop.systemd1";
  const char *path = "/org/freedesktop/systemd1";
  const char *interface = "org.freedesktop.systemd1.Manager";
  const char *system_state = "SystemState";

  if (sd_bus_default_system(&bus) < 0) {
    fprintf(stderr, "reboot:: Failed to find systemd system bus\n");
    return false;
  }

  if (sd_bus_get_property_string(bus, service, path, interface, system_state, &error, &cur_state) <
      0) {
    // System bus has often disappeared by this point when shutting down. Investigate a better
    // method of detecting shutdown
    if (strcmp(state, "stopping") == 0) {
      sd_bus_error_free(&error);
      return true;
    }
    fprintf(stderr, "reboot:: Failed to get system state: %s\n", error.name);
    sd_bus_error_free(&error);
    return false;
  }

  if (strcmp(state, cur_state) != 0) {
    free(cur_state);
    return false;
  }

  free(cur_state);
  return true;
}

/**
 * @brief Checks if the system is mid-upgrade
 *
 * @param handle reboot plugin handle
 * @return true System is upgrading
 * @return false System is not
 */
static bool prv_reboot_is_upgrade(sMemfaultdPlugin *handle) {
  struct uboot_ctx *ctx;

  if (libuboot_initialize(&ctx, NULL) < 0) {
    fprintf(stderr, "reboot:: Cannot init libuboot library\n");
    return false;
  }

  const char *file;
  if (!memfaultd_get_string(handle->memfaultd, "reboot_plugin", "uboot_fw_env_file", &file)) {
    file = FWENV_CONFIG_FILE;
  }

  if (libuboot_read_config(ctx, file) < 0) {
    libuboot_exit(ctx);
    fprintf(stderr, "reboot:: Cannot read configuration file %s\n", file);
    return false;
  }

  if (libuboot_open(ctx) < 0) {
    fprintf(stderr, "reboot:: Failed to open libuboot configuration\n");
    libuboot_exit(ctx);
    return false;
  }

  char *ustate = libuboot_get_env(ctx, "ustate");
  if (!ustate || strcmp("1", ustate) != 0) {
    free(ustate);
    libuboot_close(ctx);
    libuboot_exit(ctx);
    return false;
  }

  free(ustate);
  libuboot_close(ctx);
  libuboot_exit(ctx);
  return true;
}

/**
 * @brief Destroys reboot plugin
 *
 * @param memfaultd reboot plugin handle
 */
static void prv_reboot_destroy(sMemfaultdPlugin *handle) {
  if (handle) {
    if (prv_reboot_is_systemd_state(handle, "stopping")) {
      if (prv_reboot_is_upgrade(handle)) {
        prv_reboot_write_reboot_reason(handle, REBOOTREASON_SOFTWAREUPDATE);
      } else {
        prv_reboot_write_reboot_reason(handle, REBOOTREASON_USERRESET);
      }
    }

    free(handle);
  }
}

static sMemfaultdPluginCallbackFns s_fns = {
  .plugin_destroy = prv_reboot_destroy,
};

/**
 * @brief Initialises reboot plugin
 *
 * @param memfaultd Main memfaultd handle
 * @return callbackFunctions_t Plugin function table
 */
bool memfaultd_reboot_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns) {
  bool allowed;
  if (!memfaultd_get_boolean(memfaultd, "", "enable_data_collection", &allowed) || !allowed) {
    fprintf(stderr, "reboot:: Data collection is disabled, not starting plugin.\n");
    *fns = NULL;
    return true;
  }

  sMemfaultdPlugin *handle = calloc(sizeof(sMemfaultdPlugin), 1);
  handle->memfaultd = memfaultd;

  int reboot_reason = prv_reboot_read_and_clear_reboot_reason(handle);
  if (reboot_reason == REBOOTREASON_UNKNOWN) {
    if (access(PSTORE_FILE, F_OK) == 0) {
      reboot_reason = REBOOTREASON_KERNELPANIC;
    } else if (prv_reboot_is_systemd_state(handle, "starting")) {
      reboot_reason = REBOOTREASON_LOWPOWER;
    }
  }

  if (reboot_reason != REBOOTREASON_UNKNOWN) {
    uint32_t payload_size;
    sMemfaultdTxData *data = prv_reboot_build_event(handle, reboot_reason, NULL, &payload_size);
    if (data) {
      if (!memfaultd_txdata(handle->memfaultd, data, payload_size)) {
        fprintf(stderr, "reboot:: Failed to queue reboot reason\n");
      }
      free(data);
    }
  }

  *fns = &s_fns;
  (*fns)->handle = handle;

  return true;
}
