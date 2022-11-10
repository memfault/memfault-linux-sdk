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

#include <errno.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <systemd/sd-bus.h>
#include <unistd.h>
#include <uuid/uuid.h>

#include "memfault/core/math.h"
#include "memfault/core/reboot_reason_types.h"
#include "memfault/util/linux_boot_id.h"
#include "memfaultd.h"
#include "reboot_last_boot_id.h"
#include "reboot_process_pstore.h"

#define FWENV_CONFIG_FILE "/etc/fw_env.config"
#define PSTORE_DMESG_FILE "/sys/fs/pstore/dmesg-ramoops-0"

struct MemfaultdPlugin {
  sMemfaultd *memfaultd;
};

typedef struct MemfaultdRebootReasonSource {
  bool (*read_and_clear)(sMemfaultd *memfaultd, eMemfaultRebootReason *reboot_reason);
  const char *name;
} sMemfaultdRebootReasonSource;

static const char *prv_reboot_reason_str(eMemfaultRebootReason reboot_reason) {
  switch (reboot_reason) {
    case kMfltRebootReason_Unknown:
      return "Unknown";
    case kMfltRebootReason_UserShutdown:
      return "UserShutdown";
    case kMfltRebootReason_UserReset:
      return "UserReset";
    case kMfltRebootReason_FirmwareUpdate:
      return "FirmwareUpdate";
    case kMfltRebootReason_LowPower:
      return "LowPower";
    case kMfltRebootReason_DebuggerHalted:
      return "DebuggerHalted";
    case kMfltRebootReason_ButtonReset:
      return "ButtonReset";
    case kMfltRebootReason_PowerOnReset:
      return "PowerOnReset";
    case kMfltRebootReason_SoftwareReset:
      return "SoftwareReset";
    case kMfltRebootReason_DeepSleep:
      return "DeepSleep";
    case kMfltRebootReason_PinReset:
      return "PinReset";
    case kMfltRebootReason_UnknownError:
      return "UnknownError";
    case kMfltRebootReason_Assert:
      return "Assert";
    case kMfltRebootReason_WatchdogDeprecated:
      return "WatchdogDeprecated";
    case kMfltRebootReason_BrownOutReset:
      return "BrownOutReset";
    case kMfltRebootReason_Nmi:
      return "Nmi";
    case kMfltRebootReason_HardwareWatchdog:
      return "HardwareWatchdog";
    case kMfltRebootReason_SoftwareWatchdog:
      return "SoftwareWatchdog";
    case kMfltRebootReason_ClockFailure:
      return "ClockFailure";
    case kMfltRebootReason_KernelPanic:
      return "KernelPanic";
    case kMfltRebootReason_FirmwareUpdateError:
      return "FirmwareUpdateError";
    case kMfltRebootReason_BusFault:
      return "BusFault";
    case kMfltRebootReason_MemFault:
      return "MemFault";
    case kMfltRebootReason_UsageFault:
      return "UsageFault";
    case kMfltRebootReason_HardFault:
      return "HardFault";
    case kMfltRebootReason_Lockup:
      return "Lockup";
    default:
      return "???";
  }
}

/**
 * @brief Builds event JSON object for posting to events API
 *
 * @param handle reboot plugin handle
 * @param reason reboot reason number to encode
 * @param userinfo optional userinfo string
 * @param payload_size size of the payload in the returned data object
 * @return sMemfaultdTxData* Tx data with the reboot event
 */
static sMemfaultdTxData *prv_reboot_build_event(sMemfaultd *memfaultd,
                                                const eMemfaultRebootReason reason,
                                                const char *userinfo, uint32_t *payload_size) {
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(memfaultd);

  const char *software_type = "";
  const char *software_version = "";
  memfaultd_get_string(memfaultd, "", "software_type", &software_type);
  memfaultd_get_string(memfaultd, "", "software_version", &software_version);

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
                           settings->hardware_version, reason, userinfo ? userinfo : "");
  if (ret >= (int)max_event_size || ret < 0) {
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
static void prv_reboot_write_reboot_reason(sMemfaultd *memfaultd,
                                           eMemfaultRebootReason reboot_reason) {
  char *file = memfaultd_generate_rw_filename(memfaultd, "lastrebootreason");
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
 * @brief Reads reboot reason from given file and then deletes it
 *
 * @param char *file The file to read the reason from
 * @return int reboot_reason read from file
 */
static bool prv_reboot_read_and_clear_reboot_reason_from_file(
  const char *file, eMemfaultRebootReason *reboot_reason_out) {
  FILE *const fd = fopen(file, "r");
  if (!fd) {
    fprintf(stderr, "reboot:: Failed to open %s: %s\n", file, strerror(errno));
    return false;
  }

  bool result = false;
  int reboot_reason = 0;
  if (fscanf(fd, "%d", &reboot_reason) == 1) {
    *reboot_reason_out = reboot_reason;
    result = true;
  } else {
    fprintf(stderr, "reboot:: Failed to parse reboot reason in %s\n", file);
  }

  fclose(fd);
  unlink(file);

  return result;
}

/**
 * @brief Reads reboot reason from internal file and then deletes it
 *
 * @param handle reboot plugin handle
 * @return int reboot_reason read from file
 */
static bool prv_reboot_read_and_clear_reboot_reason_internal(sMemfaultd *memfaultd,
                                                             eMemfaultRebootReason *reboot_reason) {
  char *file = memfaultd_generate_rw_filename(memfaultd, "lastrebootreason");
  if (!file) {
    fprintf(stderr, "reboot:: Failed to allocate lastrebootreason path string\n");
    return false;
  }
  const bool result = prv_reboot_read_and_clear_reboot_reason_from_file(file, reboot_reason);
  free(file);
  return result;
}

/**
 * @brief Reads reboot reason from customer's file and then deletes it
 *
 * @param handle reboot plugin handle
 * @return int reboot_reason read from file
 */
static bool prv_reboot_read_and_clear_reboot_reason_customer(sMemfaultd *memfaultd,
                                                             eMemfaultRebootReason *reboot_reason) {
  const char *file;
  if (!memfaultd_get_string(memfaultd, "reboot_plugin", "last_reboot_reason_file", &file)) {
    fprintf(stderr, "reboot:: Failed to get configuration option last_reboot_reason_file\n");
    return false;
  }
  return prv_reboot_read_and_clear_reboot_reason_from_file(file, reboot_reason);
}

/**
 * @brief Checks if the current systemd state matches the requested state
 *
 * @param handle reboot plugin handle
 * @param state State to validate against
 * @return true Machine is in requested state
 * @return false Machine is not
 */
static bool prv_reboot_is_systemd_state(const char *state) {
  bool result = false;
  sd_bus *bus = NULL;
  sd_bus_error error = SD_BUS_ERROR_NULL;
  char *cur_state = NULL;

  const char *service = "org.freedesktop.systemd1";
  const char *path = "/org/freedesktop/systemd1";
  const char *interface = "org.freedesktop.systemd1.Manager";
  const char *system_state = "SystemState";

  const int bus_result = sd_bus_default_system(&bus);
  if (bus_result < 0) {
    fprintf(stderr, "reboot:: Failed to find systemd system bus: %s\n", strerror(-bus_result));
    goto cleanup;
  }

  if (sd_bus_get_property_string(bus, service, path, interface, system_state, &error, &cur_state) <
      0) {
    fprintf(stderr, "reboot:: Failed to get system state: %s\n", error.name);
    goto cleanup;
  }

  result = (strcmp(state, cur_state) == 0);

cleanup:
  free(cur_state);
  sd_bus_error_free(&error);
  sd_bus_unref(bus);
  return result;
}

/**
 * @brief Checks if the system is mid-upgrade
 *
 * @param handle reboot plugin handle
 * @return true System is upgrading
 * @return false System is not
 */
static bool prv_reboot_is_upgrade(sMemfaultd *memfaultd) {
  struct uboot_ctx *ctx;

  if (libuboot_initialize(&ctx, NULL) < 0) {
    fprintf(stderr, "reboot:: Cannot init libuboot library\n");
    return false;
  }

  const char *file;
  if (!memfaultd_get_string(memfaultd, "reboot_plugin", "uboot_fw_env_file", &file)) {
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
    if (prv_reboot_is_systemd_state("stopping")) {
      sMemfaultd *const memfaultd = handle->memfaultd;
      if (prv_reboot_is_upgrade(memfaultd)) {
        prv_reboot_write_reboot_reason(memfaultd, kMfltRebootReason_FirmwareUpdate);
      } else {
        prv_reboot_write_reboot_reason(memfaultd, kMfltRebootReason_UserReset);
      }
    }

    free(handle);
  }
}

static bool prv_reboot_read_and_clear_reboot_reason_pstore(sMemfaultd *memfaultd,
                                                           eMemfaultRebootReason *reboot_reason) {
  if (access(PSTORE_DMESG_FILE, F_OK) != 0) {
    return false;
  }
  memfault_reboot_process_pstore_files(PSTORE_DIR);
  *reboot_reason = kMfltRebootReason_KernelPanic;
  return true;
}

// This list is ordered by priority (high to low):
static const sMemfaultdRebootReasonSource s_reboot_reason_sources[] = {
  {
    .name = "pstore",
    .read_and_clear = prv_reboot_read_and_clear_reboot_reason_pstore,
  },
  {
    .name = "custom",
    .read_and_clear = prv_reboot_read_and_clear_reboot_reason_customer,
  },
  {
    .name = "internal",
    .read_and_clear = prv_reboot_read_and_clear_reboot_reason_internal,
  },
};

static eMemfaultRebootReason prv_resolve_reboot_reason(sMemfaultd *memfaultd, const char *boot_id) {
  // Read & clear all reboot reason sources. s_reboot_reason_sources is ordered by priority (high
  // to low). The first non-unknown reason found is used as the reason to report.

  eMemfaultRebootReason prioritized_reason = kMfltRebootReason_Unknown;
  bool prioritized_reason_set = false;

  for (size_t i = 0; i < MEMFAULT_ARRAY_SIZE(s_reboot_reason_sources); ++i) {
    eMemfaultRebootReason reason = kMfltRebootReason_Unknown;
    if (s_reboot_reason_sources[i].read_and_clear(memfaultd, &reason)) {
      if (!prioritized_reason_set) {
        prioritized_reason = reason;
        prioritized_reason_set = true;
        fprintf(stderr, "reboot:: Using reboot reason %s (0x%04x) from %s source for boot_id %s\n",
                prv_reboot_reason_str(reason), reason, s_reboot_reason_sources[i].name, boot_id);
      } else {
        fprintf(stderr,
                "reboot:: Discarded reboot reason %s (0x%04x) from %s source for boot_id %s\n",
                prv_reboot_reason_str(reason), reason, s_reboot_reason_sources[i].name, boot_id);
      }
    }
  }
  return prioritized_reason;
}

static void prv_track_reboot(sMemfaultd *memfaultd, const char *boot_id) {
  const eMemfaultRebootReason reboot_reason = prv_resolve_reboot_reason(memfaultd, boot_id);

  uint32_t payload_size;
  sMemfaultdTxData *data = prv_reboot_build_event(memfaultd, reboot_reason, NULL, &payload_size);
  if (data) {
    if (!memfaultd_txdata(memfaultd, data, payload_size)) {
      fprintf(stderr, "reboot:: Failed to queue reboot reason\n");
    }
    free(data);
  }
}

static sMemfaultdPluginCallbackFns s_fns = {
  .plugin_destroy = prv_reboot_destroy,
};

static void prv_run_if_untracked_boot_id(sMemfaultd *memfaultd,
                                         void (*cb)(sMemfaultd *memfaultd, const char *boot_id)) {
  char *last_tracked_boot_id_file =
    memfaultd_generate_rw_filename(memfaultd, "last_tracked_boot_id");
  if (last_tracked_boot_id_file == NULL) {
    fprintf(stderr, "reboot:: Failed to get last_tracked_boot_id file\n");
    goto cleanup;
  }

  char boot_id[UUID_STR_LEN];
  if (!memfault_linux_boot_id_read(boot_id)) {
    fprintf(stderr, "reboot:: Failed to get current boot_id\n");
    goto cleanup;
  }

  if (memfault_reboot_is_untracked_boot_id(last_tracked_boot_id_file, boot_id)) {
    cb(memfaultd, boot_id);
  }

cleanup:
  free(last_tracked_boot_id_file);
}

static void prv_noop_untracked_boot_id_handler(sMemfaultd *memfaultd, const char *boot_id) {}

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

    // If data collection is disabled, mark the current boot as tracked (if it has not already
    // been done so), without emitting a reboot event, because the reboot happened before data
    // collection was enabled:
    prv_run_if_untracked_boot_id(memfaultd, prv_noop_untracked_boot_id_handler);

    // Always process the pstore, even if data collection is disabled:
    memfault_reboot_process_pstore_files(PSTORE_DIR);
    return true;
  }

  sMemfaultdPlugin *handle = calloc(sizeof(sMemfaultdPlugin), 1);
  handle->memfaultd = memfaultd;

  *fns = &s_fns;
  (*fns)->handle = handle;

  prv_run_if_untracked_boot_id(memfaultd, prv_track_reboot);

  return true;
}
