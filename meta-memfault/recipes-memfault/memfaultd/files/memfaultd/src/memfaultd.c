//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Core functionality and main loop

#include "memfaultd.h"

#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <json-c/json.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/types.h>
#include <unistd.h>

#include "config.h"
#include "device_settings.h"
#include "network.h"
#include "queue.h"

#define PID_FILE "/var/run/memfaultd.pid"
#define CONFIG_FILE "/etc/memfaultd.conf"

struct Memfaultd {
  sMemfaultdQueue *queue;
  sMemfaultdNetwork *network;
  sMemfaultdConfig *config;
  sMemfaultdDeviceSettings *settings;
  bool terminate;
};

typedef struct {
  memfaultd_plugin_init init;
  sMemfaultdPluginCallbackFns *fns;
  const char name[32];
} sMemfaultdPluginDef;

#ifdef PLUGIN_REBOOT
const bool memfaultd_reboot_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif

#ifdef PLUGIN_SWUPDATE
const bool memfaultd_swupdate_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif

static sMemfaultdPluginDef s_plugins[] = {
#ifdef PLUGIN_REBOOT
  {.name = "reboot", .init = memfaultd_reboot_init},
#endif
#ifdef PLUGIN_SWUPDATE
  {.name = "swupdate", .init = memfaultd_swupdate_init},
#endif
};

static sMemfaultd *s_handle;

#ifndef VERSION
  #define VERSION dev
#endif
#ifndef GIT_HASH
  #define GIT_HASH unknown
#endif

#define STRINGIZE(x) #x
#define STRINGIZE_VALUE_OF(x) STRINGIZE(x)

/**
 * @brief Displays SDK version information
 *
 */
static void prv_memfaultd_version_info(void) {
  printf("VERSION=%s\n", STRINGIZE_VALUE_OF(VERSION));
  printf("GIT_HASH=%s\n", STRINGIZE_VALUE_OF(GIT_HASH));
}

/**
 * @brief Displays usage information
 *
 */
static void prv_memfaultd_usage(void) {
  printf("Usage: memfaultd [OPTION]...\n\n");
  printf("  -c, --config-file <file>      : Configuration file\n");
  printf("  -d, --daemonize               : Daemonize process\n");
  printf("  -e, --enable-data-collection  : Enable automatic collection, will restart the main "
         "memfaultd service\n");
  printf("  -h, --help                    : Display this help and exit\n");
  printf("  -s, --show-settings           : Show settings\n");
  printf("  -v, --version                 : Show version information\n");
}

/**
 * @brief Call the init function of all defined plugins
 *
 * @param handle Main memfaultd handle
 */
static void prv_memfaultd_load_plugins(sMemfaultd *handle) {
  for (int i = 0; i < sizeof(s_plugins) / sizeof(sMemfaultdPluginDef); ++i) {
    if (!s_plugins[i].init(handle, &s_plugins[i].fns)) {
      fprintf(stderr, "memfaultd:: Failed to initialize %s plugin, destroying.\n",
              s_plugins[i].name);
      if (s_plugins[i].fns->plugin_destroy) {
        s_plugins[i].fns->plugin_destroy(s_plugins[i].fns->handle);
      }
    }
  }
}

/**
 * @brief Call the destroy function of all defined plugins
 */
static void prv_memfaultd_destroy_plugins(void) {
  for (int i = 0; i < sizeof(s_plugins) / sizeof(sMemfaultdPluginDef); ++i) {
    if (s_plugins[i].fns != NULL && s_plugins[i].fns->plugin_destroy) {
      s_plugins[i].fns->plugin_destroy(s_plugins[i].fns->handle);
    }
  }
}

/**
 * @brief Enable collection in config and start daemon
 *
 * @param handle Main memfaultd handle
 */
static void prv_memfaultd_enable_collection(sMemfaultd *handle) {
  bool enabled = false;
  if (memfaultd_get_boolean(handle, "", "enable_data_collection", &enabled) && enabled) {
    printf("Data collection already enabled\n");
    return;
  }
  printf("Enabling data collection\n");
  memfaultd_set_boolean(handle, "", "enable_data_collection", true);

  if (getuid() == 0 && system("/bin/systemctl restart memfaultd.service") != 0) {
    fprintf(stderr, "memfaultd:: Failed to restart memfaultd.\n");
  }
}

/**
 * @brief Signal handler
 *
 * @param sig Signal number
 */
static void prv_memfaultd_sig_handler(int sig) { s_handle->terminate = true; }

/**
 * @brief Looks up the HTTP API path given a eMemfaultdTxDataType.
 * @param type The type to lookup.
 * @return The HTTP API path or NULL if the type is unknown.
 */
static const char *prv_endpoint_for_txdata_type(uint8_t type) {
  switch (type) {
    case kMemfaultdTxDataType_RebootEvent:
      return "/api/v0/events";
    default:
      return NULL;
  }
}

/**
 * @brief Process TX queue and transmit messages
 *
 * @param handle Main memfaultd handle
 */
static void prv_memfaultd_process_tx_queue(sMemfaultd *handle) {
  bool allowed;
  if (!memfaultd_get_boolean(handle, NULL, "enable_data_collection", &allowed) || !allowed) {
    return;
  }

  uint32_t payload_size_bytes;
  uint8_t *payload;
  while ((payload = memfaultd_queue_read_head(handle->queue, &payload_size_bytes))) {
    const sMemfaultdTxData *txdata = (const sMemfaultdTxData *)payload;
    const char *endpoint = prv_endpoint_for_txdata_type(txdata->type);
    // FIXME: assuming NUL terminated UTF-8 C string -- need to support binary data too:
    const char *request_body = (const char *)txdata->payload;
    if (endpoint == NULL ||
        memfaultd_network_post(handle->network, endpoint, request_body, NULL, 0)) {
      free(payload);
      memfaultd_queue_complete_read(handle->queue);
    } else {
      free(payload);
      return;
    }
  }
}

/**
 * @brief Checks for memfaultd daemon PID file
 *
 * @return true PID file exists
 * @return false Does not exist
 */
static bool prv_memfaultd_check_for_pid_file(void) {
  const int fd = open(PID_FILE, O_WRONLY | O_EXCL, S_IRUSR | S_IWUSR);
  if (fd == -1) {
    if (errno == ENOENT) {
      return false;
    } else {
      // PID file exists, but can't open it for some reason
      return true;
    }
  }

  close(fd);
  return true;
}

/**
 * @brief Daemonize process
 *
 * @return true Successfully daemonized
 * @return false Failed
 */
static bool prv_memfaultd_daemonize_process(void) {
  char pid[11] = "";
  if (getuid() != 0) {
    fprintf(stderr, "memfaultd:: Cannot daemonize as non-root user, aborting.\n");
  }

  // Use noclose=1 so logs written to stdout/stderr can be viewed using journalctl
  if (daemon(0, 1) == -1) {
    fprintf(stderr, "memfaultd:: Failed to daemonize, aborting.\n");
    return false;
  }

  const int fd = open(PID_FILE, O_WRONLY | O_CREAT | O_EXCL, S_IRUSR | S_IWUSR);
  if (fd == -1) {
    if (errno == EEXIST) {
      fprintf(stderr, "memfaultd:: Daemon already running, aborting.\n");
      return false;
    }
    fprintf(stderr, "memfaultd:: Failed to open PID file, aborting.\n");
    return false;
  }

  snprintf(pid, sizeof(pid), "%d\n", getpid());

  if (write(fd, pid, sizeof(pid)) == -1) {
    fprintf(stderr, "memfaultd:: Failed to write PID file, aborting.\n");
    close(fd);
    unlink(PID_FILE);
  }

  close(fd);

  return true;
}

/**
 * @brief Main process loop
 *
 * @param handle Main memfaultd handle
 */
static void prv_memfaultd_process_loop(sMemfaultd *handle) {
  while (!handle->terminate) {
    struct timeval last_wakeup;
    gettimeofday(&last_wakeup, NULL);

    prv_memfaultd_process_tx_queue(handle);

    int interval;
    if (!memfaultd_get_integer(handle, NULL, "refresh_interval_seconds", &interval)) {
      interval = 60 * 60;
      fprintf(stderr, "memfaultd:: No network update interval defined, defaulting to %u.\n",
              interval);
    }

    struct timeval now;
    gettimeofday(&now, NULL);

    if (!handle->terminate && last_wakeup.tv_sec + interval > now.tv_sec) {
      sleep(last_wakeup.tv_sec + interval - now.tv_sec);
    }
  }
}

static void memfaultd_dump_config(sMemfaultd *handle, const char *config_file) {
  memfaultd_config_dump_config(handle->config, config_file);

  printf("Device configuration from memfault-device-info:\n");
  printf("  MEMFAULT_DEVICE_ID=%s\n", handle->settings->device_id);
  printf("  MEMFAULT_HARDWARE_VERSION=%s\n", handle->settings->hardware_version);
  printf("\n");

  printf("Plugin enabled:\n");
  for (int i = 0; i < sizeof(s_plugins) / sizeof(sMemfaultdPluginDef); ++i) {
    printf("  %s\n", s_plugins[i].name);
  }
}

static void memfaultd_create_data_dir(sMemfaultd *handle) {
  const char *data_dir;
  if (!memfaultd_get_string(handle, "", "data_dir", &data_dir) || strlen(data_dir) == 0) {
    return;
  }

  struct stat sb;
  if (stat(data_dir, &sb) == 0 && S_ISDIR(sb.st_mode)) {
    return;
  }

  if (mkdir(data_dir, 0755) == -1) {
    fprintf(stderr, "memfault:: Failed to create memfault base_dir '%s'\n", data_dir);
    return;
  }
}

/**
 * @brief Entry function
 *
 * @param argc Argument count
 * @param argv Argument array
 * @return int Return code
 */
int main(int argc, char *argv[]) {
  bool daemonize = false;
  bool enable = false;
  bool display_config = false;

  s_handle = calloc(sizeof(sMemfaultd), 1);
  const char *config_file = CONFIG_FILE;

  static struct option sMemfaultdLongOptions[] = {{"config-file", required_argument, NULL, 'c'},
                                                  {"daemonize", no_argument, NULL, 'd'},
                                                  {"enable-collection", no_argument, NULL, 'e'},
                                                  {"help", no_argument, NULL, 'h'},
                                                  {"show-settings", no_argument, NULL, 's'},
                                                  {"version", no_argument, NULL, 'v'},
                                                  {NULL, 0, NULL, 0}};

  int opt;
  while ((opt = getopt_long(argc, argv, "c:dehsv", sMemfaultdLongOptions, NULL)) != -1) {
    switch (opt) {
      case 'c':
        config_file = optarg;
        break;
      case 'd':
        daemonize = true;
        break;
      case 'e':
        enable = true;
        break;
      case 'h':
        prv_memfaultd_usage();
        exit(EXIT_SUCCESS);
      case 's':
        display_config = true;
        break;
      case 'v':
        prv_memfaultd_version_info();
        exit(EXIT_SUCCESS);
      default:
        exit(EXIT_FAILURE);
    }
  }

  if (!(s_handle->config = memfaultd_config_init(s_handle, config_file))) {
    fprintf(stderr, "memfaultd:: Failed to create config object, aborting.\n");
    exit(EXIT_FAILURE);
  }

  memfaultd_create_data_dir(s_handle);

  if (enable) {
    prv_memfaultd_enable_collection(s_handle);
    exit(EXIT_SUCCESS);
  }

  if (!daemonize && prv_memfaultd_check_for_pid_file()) {
    fprintf(stderr, "memfaultd:: memfaultd already running, pidfile: '%s'.\n", PID_FILE);
    exit(EXIT_FAILURE);
  }

  if (!(s_handle->settings = memfaultd_device_settings_init())) {
    fprintf(stderr, "memfaultd:: Failed to load all required device settings, aborting.\n");
    exit(EXIT_FAILURE);
  }

  memfaultd_dump_config(s_handle, config_file);
  if (display_config) {
    /* Already reported above, just exit */
    exit(EXIT_SUCCESS);
  }

  signal(SIGTERM, prv_memfaultd_sig_handler);
  signal(SIGHUP, prv_memfaultd_sig_handler);
  signal(SIGINT, prv_memfaultd_sig_handler);

  int queue_size = 0;
  memfaultd_get_integer(s_handle, NULL, "queue_size_kib", &queue_size);

  if (!(s_handle->queue = memfaultd_queue_init(s_handle, queue_size * 1024))) {
    fprintf(stderr, "memfaultd:: Failed to create queue object, aborting.\n");
    exit(EXIT_FAILURE);
  }

  bool allowed;
  if (!memfaultd_get_boolean(s_handle, NULL, "enable_data_collection", &allowed) || !allowed) {
    memfaultd_queue_reset(s_handle->queue);
  }

  if (!(s_handle->network = memfaultd_network_init(s_handle))) {
    fprintf(stderr, "memfaultd:: Failed to create networking object, aborting.\n");
    exit(EXIT_FAILURE);
  }

  prv_memfaultd_load_plugins(s_handle);

  if (daemonize && !prv_memfaultd_daemonize_process()) {
    exit(EXIT_FAILURE);
  }

  prv_memfaultd_process_loop(s_handle);

  prv_memfaultd_destroy_plugins();

  memfaultd_network_destroy(s_handle->network);
  memfaultd_queue_destroy(s_handle->queue);
  memfaultd_config_destroy(s_handle->config);
  memfaultd_device_settings_destroy(s_handle->settings);
  free(s_handle);

  if (daemonize) {
    unlink(PID_FILE);
  }
  return 0;
}

/**
 * @brief Plugin API impl for queueing data to transmit
 *
 * @param handle Main memfaultd handle
 * @param data Data to transmit
 * @param payload_size Size of the data->payload buffer
 * @return true Successfully queued for transmitting
 * @return false Failed to queue
 */
bool memfaultd_txdata(sMemfaultd *handle, const sMemfaultdTxData *data, uint32_t payload_size) {
  bool allowed;
  if (!memfaultd_get_boolean(handle, "", "enable_data_collection", &allowed) || !allowed) {
    return true;
  }
  return memfaultd_queue_write(handle->queue, (const uint8_t *)data,
                               sizeof(sMemfaultdTxData) + payload_size);
}

void memfaultd_set_boolean(sMemfaultd *handle, const char *parent_key, const char *key,
                           const bool val) {
  memfaultd_config_set_boolean(handle->config, parent_key, key, val);
}
void memfaultd_set_integer(sMemfaultd *handle, const char *parent_key, const char *key,
                           const int val) {
  memfaultd_config_set_integer(handle->config, parent_key, key, val);
}
void memfaultd_set_string(sMemfaultd *handle, const char *parent_key, const char *key,
                          const char *val) {
  memfaultd_config_set_string(handle->config, parent_key, key, val);
}
bool memfaultd_get_boolean(sMemfaultd *handle, const char *parent_key, const char *key, bool *val) {
  return memfaultd_config_get_boolean(handle->config, parent_key, key, val);
}
bool memfaultd_get_integer(sMemfaultd *handle, const char *parent_key, const char *key, int *val) {
  return memfaultd_config_get_integer(handle->config, parent_key, key, val);
}
bool memfaultd_get_string(sMemfaultd *handle, const char *parent_key, const char *key,
                          const char **val) {
  return memfaultd_config_get_string(handle->config, parent_key, key, val);
}

bool memfaultd_get_objects(sMemfaultd *handle, const char *parent_key,
                           sMemfaultdConfigObject **objects, int *len) {
  return memfaultd_config_get_objects(handle->config, parent_key, objects, len);
}

const sMemfaultdDeviceSettings *memfaultd_get_device_settings(sMemfaultd *memfaultd) {
  return memfaultd->settings;
}

char *memfaultd_generate_rw_filename(sMemfaultd *memfaultd, const char *filename) {
  const char *data_dir;
  char *file = NULL;
  if (memfaultd_get_string(memfaultd, "", "data_dir", &data_dir) && strlen(data_dir) != 0) {
    file = malloc(strlen(data_dir) + strlen(filename) + 1 + 1);
    if (!file) {
      return NULL;
    }
    strcpy(file, data_dir);
    strcat(file, "/");
    strcat(file, filename);
  }

  return file;
}
