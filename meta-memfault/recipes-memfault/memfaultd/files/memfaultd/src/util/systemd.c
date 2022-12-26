//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd systemd helper

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <systemd/sd-bus.h>

static const char *const systemd_service = "org.freedesktop.systemd1";

/**
 * @param state output variable for current state (to be freed by caller).
 */
static bool prv_systemd_get_service_state(sd_bus *bus, const char *service_name, char **state) {
  static const char *const unit_interface = "org.freedesktop.systemd1.Unit";

  char *unit_path = NULL;
  bool result = true;

  if (sd_bus_path_encode("/org/freedesktop/systemd1/unit", service_name, &unit_path) < 0) {
    fprintf(stderr, "memfaultd:: Failed to generate SystemD unit path\n");

    return false;
  }

  sd_bus_error error = SD_BUS_ERROR_NULL;
  if (sd_bus_get_property_string(bus, systemd_service, unit_path, unit_interface, "ActiveState",
                                 &error, state) < 0) {
    fprintf(stderr, "memfaultd:: Failed to get state of %s: %s\n", service_name, error.name);
    sd_bus_error_free(&error);
    result = false;
  }

  if (unit_path) {
    free(unit_path);
  }
  return result;
}

/*
 * List of SystemD commands: https://www.freedesktop.org/wiki/Software/systemd/dbus/
 */

static bool prv_systemd_restart_service(sd_bus *bus, const char *service_name) {
  const char *manager_path = "/org/freedesktop/systemd1";
  const char *manager_interface = "org.freedesktop.systemd1.Manager";

  sd_bus_error error = SD_BUS_ERROR_NULL;
  sd_bus_message *msg = NULL;

  if (sd_bus_call_method(bus, systemd_service, manager_path, manager_interface, "RestartUnit",
                         &error, &msg, "ss", service_name, "replace") < 0) {
    fprintf(stderr, "memfaultd:: Failed to restart %s: %s\n", service_name, error.name);
    sd_bus_error_free(&error);
    return false;
  }

  sd_bus_message_unref(msg);

  return true;
}

static bool prv_systemd_kill_service(sd_bus *bus, const char *service_name, int signal) {
  const char *manager_path = "/org/freedesktop/systemd1";
  const char *manager_interface = "org.freedesktop.systemd1.Manager";

  sd_bus_error error = SD_BUS_ERROR_NULL;
  sd_bus_message *msg = NULL;

  /*
    Refer to SystemD Bus documentation for arguments to the call:
      KillUnit(in  s name,
               in  s who,       // "all": is the default (like systemctl kill service)
               in  i signal);
  */
  if (sd_bus_call_method(bus, systemd_service, manager_path, manager_interface, "KillUnit", &error,
                         &msg, "ssi", service_name, "all", signal) < 0) {
    fprintf(stderr, "memfaultd:: Failed to kill %s: %s\n", service_name, error.name);
    sd_bus_error_free(&error);
    return false;
  }

  sd_bus_message_unref(msg);

  return true;
}

/**
 * @brief Restart service using systemd dbus API if already running
 *
 * @param src_module Friendly name of caller module
 * @param service_name Service name to restart, e.g. collectd.service
 * @return true Successfully restarted requested service
 * @return false Failed to restart service
 */
bool memfaultd_restart_service_if_running(const char *service_name) {
  bool result = true;

  // Initialize connection to SystemD
  sd_bus *bus;
  if (sd_bus_default_system(&bus) < 0) {
    fprintf(stderr, "memfaultd:: Failed to find systemd system bus\n");
    return false;
  }

  // Check if service is active before restarting it
  char *state = NULL;
  if (!prv_systemd_get_service_state(bus, service_name, &state)) {
    result = false;
    goto cleanup;
  }
  if (strcmp("active", state) != 0 && strcmp("activating", state) != 0) {
    fprintf(stderr, "memfaultd:: %s is not active (%s). Not starting.\n", service_name, state);
    result = false;
    goto cleanup;
  }

  // Restart the service
  if (!prv_systemd_restart_service(bus, service_name)) {
    result = false;
    goto cleanup;
  }

cleanup:
  if (state != NULL) {
    free(state);
  }
  sd_bus_unref(bus);
  return result;
}

/**
 * @brief Send a signal to a service if it's running.
 *
 * @param src_module Friendly name of caller module
 * @param service_name Service name to "kill", e.g. collectd.service
 * @param signal Signal to send, e.g. SIGUSR1
 * @return true Successfully sent signal to requested service
 * @return false Failed to restart service
 */
bool memfaultd_kill_service(const char *service_name, int signal) {
  bool result = true;

  // Initialize connection to SystemD
  sd_bus *bus;
  if (sd_bus_default_system(&bus) < 0) {
    fprintf(stderr, "memfaultd:: Failed to find systemd system bus\n");
    return false;
  }

  // Send signal to service
  if (!prv_systemd_kill_service(bus, service_name, signal)) {
    result = false;
    goto cleanup;
  }

cleanup:
  sd_bus_unref(bus);
  return result;
}
