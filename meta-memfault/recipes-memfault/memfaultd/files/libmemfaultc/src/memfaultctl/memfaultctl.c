//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultctl implementation

#include "memfaultctl.h"

#include <assert.h>
#include <errno.h>
#include <json-c/json.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "crash.h"
#include "memfault/core/math.h"
#include "memfault/util/config.h"
#include "memfault/util/device_settings.h"
#include "memfault/util/dump_settings.h"
#include "memfault/util/ipc.h"
#include "memfault/util/plugins.h"
#include "memfault/util/reboot_reason.h"
#include "memfault/util/runtime_config.h"
#include "memfault/util/systemd.h"
#include "memfault/util/version.h"
#include "parse_attributes.h"

static void prv_memfaultctl_usage(void);

typedef struct MemfaultCtl {
  char *config_file;
  sMemfaultdConfig *config;
  sMemfaultdDeviceSettings *settings;
  char **extra_args;
  int extra_args_count;
  bool dev_mode;
} sMemfaultCtl;

static int prv_cmd_show_settings(sMemfaultCtl *handle) {
  memfaultd_dump_settings(handle->settings, handle->config, handle->config_file);
  return 0;
}

static int prv_cmd_enable_developer_mode(sMemfaultCtl *h) {
  return memfault_set_runtime_bool_and_reload(h->config, CONFIG_KEY_DEV_MODE, "developer mode",
                                              true);
}
static int prv_cmd_disable_developer_mode(sMemfaultCtl *h) {
  return memfault_set_runtime_bool_and_reload(h->config, CONFIG_KEY_DEV_MODE, "developer mode",
                                              false);
}
static int prv_cmd_enable_data_collection(sMemfaultCtl *h) {
  return memfault_set_runtime_bool_and_reload(h->config, CONFIG_KEY_DATA_COLLECTION,
                                              "data collection", true);
}
static int prv_cmd_disable_data_collection(sMemfaultCtl *h) {
  return memfault_set_runtime_bool_and_reload(h->config, CONFIG_KEY_DATA_COLLECTION,
                                              "data collection", false);
}

static int prv_cmd_reboot(sMemfaultCtl *h) {
  const char *reboot_reason_file;

  if (!memfaultd_config_get_string(h->config, "reboot_plugin", "last_reboot_reason_file",
                                   &reboot_reason_file)) {
    fprintf(stderr, "Unable to read location of reboot_reason_file in configuration.\n");
    return -1;
  }

  int reboot_reason = kMfltRebootReason_Unknown;

  if (h->extra_args_count > 0) {
    if (h->extra_args_count != 2 || strcmp(h->extra_args[0], "--reason") != 0) {
      prv_memfaultctl_usage();
      return -1;
    }

    const char *reason_str = h->extra_args[1];

    if (sscanf(reason_str, "%d", &reboot_reason) <= 0 ||
        !memfaultd_is_reboot_reason_valid(reboot_reason)) {
      fprintf(stderr,
              "Invalid reboot reason '%s'.\n"
              "Refer to https://docs.memfault.com/docs/platform/reference-reboot-reason-ids\n",
              reason_str);
      return -1;
    }
  }

  printf("Rebooting with reason %d (%s)\n", reboot_reason,
         memfaultd_reboot_reason_str(reboot_reason));
  FILE *file = fopen(reboot_reason_file, "w");
  if (!file) {
    fprintf(stderr, "Unable to open file: %s\n", strerror(errno));
    return -1;
  }
  if (fprintf(file, "%d", reboot_reason) < 0) {
    fprintf(stderr, "Error writing reboot reason: %s\n", strerror(errno));
    fclose(file);
    return -1;
  }
  fclose(file);

  if (system("reboot") < 0) {
    fprintf(stderr, "Unable to call 'reboot': %s\n", strerror(errno));
    return -1;
  }
  return 0;
}

static int prv_cmd_request_metrics(sMemfaultCtl *h) {
#ifdef PLUGIN_COLLECTD
  return memfaultd_ipc_sendmsg((uint8_t *)PLUGIN_COLLECTD_IPC_NAME,
                               sizeof(PLUGIN_COLLECTD_IPC_NAME))
           ? 0
           : -1;
#endif
  return 0;
}

static int prv_cmd_sync(sMemfaultCtl *h) {
  // Tell memfaultd to flush the queue now. Errors will be printed to stderr.
  return memfaultd_send_flush_queue_signal() ? 0 : -1;
}

static int prv_cmd_trigger_coredump(sMemfaultCtl *h) {
#ifdef PLUGIN_COREDUMP
  eErrorType e = eErrorTypeSegFault;

  if (h->extra_args_count > 0) {
    if (strcmp(h->extra_args[0], "segfault")) {
      e = eErrorTypeSegFault;
    } else if (strcmp(h->extra_args[0], "divide-by-zero")) {
      e = eErrorTypeFPException;
    } else {
      fprintf(stderr, "Unknown exception type %s. Select segfault or divide-by-zero.\n",
              h->extra_args[0]);
      return -1;
    }
  }
  // Trigger crash and upload immediately if we are in developer mode.
  printf("Triggering coredump ...\n");
  memfault_trigger_crash(e);
  if (h->dev_mode) {
    // Give the kernel and memfault-core-handler time to process the coredump
    sleep(3);

    printf("Signaling memfaultd to upload coredump event...\n");
    if (!memfaultd_send_flush_queue_signal()) {
      return -1;
    }
  }
  return 0;
#else
  printf("You must enable PLUGIN_COREDUMP when building memfault SDK to report coredumps.\n");
  return -1;
#endif
}

static int prv_cmd_write_attributes(sMemfaultCtl *h) {
  bool success;

  // Convert the arguments into a JSON object suitable for the PATCH API
  json_object *json;
  if (!memfaultd_parse_attributes((const char **)h->extra_args, h->extra_args_count, &json)) {
    fprintf(stderr, "Unable to parse attributes.\n"
                    "Expect memfaultctl write-attributes var1=value1 "
                    "var2=value2 var3=value3 ...\n");
    return -1;
  }

  const char *stringified = json_object_to_json_string(json);

  // Calculate the size of the buffer we need and allocate memory
  const size_t msg_size = sizeof(sMemfaultAttributesIPC) + strlen(stringified) + 1;
  sMemfaultAttributesIPC *msg = malloc(msg_size);
  if (!msg) {
    fprintf(stderr, "Memory allocation error.\n");
    success = false;
    goto cleanup;
  }

  // Setup an IPC message to the attributes plugin with the JSON.
  strncpy(msg->name, PLUGIN_ATTRIBUTES_IPC_NAME, sizeof(msg->name));
  assert((msg->timestamp = time(NULL)) > 0);
  strncpy(msg->json, stringified, msg_size - sizeof(sMemfaultAttributesIPC));

  // Send the data via IPC to memfaultd
  success = memfaultd_ipc_sendmsg((uint8_t *)msg, msg_size);

  if (success) {
    // Upload immediately if we are in developer mode
    if (h->dev_mode) {
      memfaultd_send_flush_queue_signal();
    } else {
      printf("Message queued.\n");
    }
  }

cleanup:
  free(msg);
  json_object_put(json);
  return success ? 0 : -1;
}

typedef struct MemfaultCmd {
  const char *name;
  int (*cmd)(sMemfaultCtl *);
  // Optional description of the arguments for this command e.g: "<n>"
  const char *example_args;
  const char *help;
} sMemfaultCmd;

static const sMemfaultCmd cmds[] = {
  {.name = "enable-data-collection",
   .cmd = prv_cmd_enable_data_collection,
   .help = "Enable data collection and restart memfaultd"},
  {.name = "disable-data-collection",
   .cmd = prv_cmd_disable_data_collection,
   .help = "Disable data collection and restart memfaultd"},
  {.name = "enable-dev-mode",
   .cmd = prv_cmd_enable_developer_mode,
   .help = "Enable developer mode and restart memfaultd"},
  {.name = "disable-dev-mode",
   .cmd = prv_cmd_disable_developer_mode,
   .help = "Disable developer mode and restart memfaultd"},
  {.name = "reboot",
   .cmd = prv_cmd_reboot,
   .example_args = "[--reason <n>]",
   .help = "Register reboot reason and call 'reboot'"},
  {.name = "request-metrics",
   .cmd = prv_cmd_request_metrics,
   .help = "Flush collectd metrics to Memfault now"},
  {.name = "show-settings", .cmd = prv_cmd_show_settings, .help = "Show memfaultd settings"},
  {.name = "sync", .cmd = prv_cmd_sync, .help = "Flush memfaultd queue to Memfault now"},
  {.name = "trigger-coredump",
   .cmd = prv_cmd_trigger_coredump,
   .example_args = "[segfault|divide-by-zero]",
   .help = "Trigger a coredump and immediately reports it to Memfault (defaults to segfault)"},
  {.name = "write-attributes",
   .cmd = prv_cmd_write_attributes,
   .example_args = "<VAR1=VAL1 ...>",
   .help = "Write device attribute(s) to Memfaultd"},
};

static void prv_memfaultctl_usage(void) {
  // Calculate the size of the longest command name + example arguments
  int format_width = 0;
  static const int extra_spacing_with_arguments = 1;
  for (unsigned long i = 0; i < MEMFAULT_ARRAY_SIZE(cmds); i++) {
    int len = strlen(cmds[i].name) + (cmds[i].example_args != NULL ? extra_spacing_with_arguments +
                                                                       strlen(cmds[i].example_args)
                                                                   : 0);
    if (len > format_width) {
      format_width = len;
    }
  }

  // Print static usage info
  printf("Usage: memfaultctl [OPTION] <COMMAND> ...\n\n");
  printf("  %-*s : Use configuration file\n", format_width, "-c <config file>");
  printf("  %-*s : Display this help and exit\n", format_width, "-h");
  printf("  %-*s : Show version information\n", format_width, "-v");
  printf("\n");
  printf("Commands:\n");

  // Print commands usage info
  for (unsigned long i = 0; i < MEMFAULT_ARRAY_SIZE(cmds); i++) {
    if (cmds[i].example_args == NULL) {
      printf("  %-*s : %s\n", format_width, cmds[i].name, cmds[i].help);
    } else {
      printf("  %s %-*s : %s\n", cmds[i].name,
             format_width - (int)strlen(cmds[i].name) - extra_spacing_with_arguments,
             cmds[i].example_args, cmds[i].help);
    }
  }
  printf("\n");
}

int memfaultctl_main(int argc, char *argv[]) {
  sMemfaultCtl handle = {.config_file = CONFIG_FILE};

  int opt;
  while ((opt = getopt(argc, argv, "+c:hv")) != -1) {
    switch (opt) {
      case 'c':
        handle.config_file = optarg;
        break;
      case 'h':
        prv_memfaultctl_usage();
        exit(0);
      case 'v':
        memfault_version_print_info();
        exit(EXIT_SUCCESS);
      // case '?': // needed?
      default:
        exit(-1);
    }
  }

  if (optind >= argc) {
    prv_memfaultctl_usage();
    exit(-1);
  }
  char *command = argv[optind];

  handle.config = memfaultd_config_init(handle.config_file);
  if (!handle.config) {
    exit(-1);
  }
  handle.settings = memfaultd_device_settings_init();
  memfaultd_config_get_boolean(handle.config, NULL, CONFIG_KEY_DEV_MODE, &handle.dev_mode);

  int retval = -1;

  handle.extra_args_count = argc - optind - 1;
  if (handle.extra_args_count > 0) {
    handle.extra_args = &argv[optind + 1];
  }

  // Run user command
  for (unsigned int i = 0; i < MEMFAULT_ARRAY_SIZE(cmds); i++) {
    if (strcmp(command, cmds[i].name) == 0) {
      retval = cmds[i].cmd(&handle);
      goto cleanup;
    }
  }

  // Or show help and exit
  prv_memfaultctl_usage();

cleanup:
  memfaultd_config_destroy(handle.config);
  memfaultd_device_settings_destroy(handle.settings);
  exit(retval);
}
