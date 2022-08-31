//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd helper util function implementation

#include <stdbool.h>
#include <stdlib.h>
#include <systemd/sd-bus.h>

/**
 * @brief Restart service using systemd dbus API if already running
 *
 * @param src_module Friendly name of caller module
 * @param service_name Service name to restart, e.g. collectd.service
 * @return true Successfully restarted requested service
 * @return false Failed to restart service
 */
bool memfaultd_utils_restart_service_if_running(const char *src_module, const char *service_name) {
  sd_bus *bus = NULL;
  sd_bus_error error = SD_BUS_ERROR_NULL;
  sd_bus_message *msg = NULL;
  bool result = true;
  char *state = NULL;
  char *unit_path = NULL;

  if (!src_module || strlen(src_module) == 0 || !service_name || strlen(service_name) == 0) {
    fprintf(stderr, "Invalid parameter into memfaultd_utils_restart_service_if_running()\n");
    return false;
  }

  const char *service = "org.freedesktop.systemd1";

  if (sd_bus_default_system(&bus) < 0) {
    fprintf(stderr, "%s:: Failed to find systemd system bus\n", src_module);
    result = false;
    goto cleanup;
  }

  sd_bus_path_encode("/org/freedesktop/systemd1/unit", service_name, &unit_path);
  if (sd_bus_path_encode("/org/freedesktop/systemd1/unit", service_name, &unit_path) < 0) {
    fprintf(stderr, "%s:: Failed to generate unit path\n", src_module);
    result = false;
    goto cleanup;
  }

  const char *unit_interface = "org.freedesktop.systemd1.Unit";
  if (sd_bus_get_property_string(bus, service, unit_path, unit_interface, "ActiveState", &error,
                                 &state) < 0) {
    fprintf(stderr, "%s:: Failed to get state of %s: %s\n", src_module, service_name, error.name);
    result = false;
    goto cleanup;
  }

  if (strcmp("active", state) != 0 && strcmp("activating", state) != 0) {
    // Service is not active, do not start
    goto cleanup;
  }

  const char *manager_path = "/org/freedesktop/systemd1";
  const char *manager_interface = "org.freedesktop.systemd1.Manager";
  if (sd_bus_call_method(bus, service, manager_path, manager_interface, "RestartUnit", &error, &msg,
                         "ss", service_name, "replace") < 0) {
    fprintf(stderr, "%s:: Failed to restart %s: %s\n", src_module, service_name, error.name);
    result = false;
    goto cleanup;
  }

cleanup:
  free(state);
  free(unit_path);
  sd_bus_error_free(&error);
  sd_bus_message_unref(msg);
  sd_bus_unref(bus);
  return result;
}
