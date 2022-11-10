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
#include <pthread.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/param.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/un.h>
#include <unistd.h>

#include "config.h"
#include "device_settings.h"
#include "memfault/core/math.h"
#include "memfault/util/disk.h"
#include "memfaultd_utils.h"
#include "network.h"
#include "queue.h"

#define SOCKET_PATH "/tmp/memfault-ipc.sock"
#define RX_BUFFER_SIZE 1024
#define PID_FILE "/var/run/memfaultd.pid"
#define CONFIG_FILE "/etc/memfaultd.conf"

struct Memfaultd {
  sMemfaultdQueue *queue;
  sMemfaultdNetwork *network;
  sMemfaultdConfig *config;
  sMemfaultdDeviceSettings *settings;
  bool terminate;
  pthread_t ipc_thread_id;
  int ipc_socket_fd;
  char ipc_rx_buffer[RX_BUFFER_SIZE];
};

typedef struct {
  memfaultd_plugin_init init;
  sMemfaultdPluginCallbackFns *fns;
  const char name[32];
  const char ipc_name[32];
} sMemfaultdPluginDef;

#ifdef PLUGIN_REBOOT
bool memfaultd_reboot_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
void memfaultd_reboot_data_collection_enabled(sMemfaultd *memfaultd, bool data_collection_enabled);
#endif
#ifdef PLUGIN_SWUPDATE
bool memfaultd_swupdate_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif
#ifdef PLUGIN_COLLECTD
bool memfaultd_collectd_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif
#ifdef PLUGIN_COREDUMP
bool memfaultd_coredump_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
#endif

static sMemfaultdPluginDef s_plugins[] = {
#ifdef PLUGIN_REBOOT
  {.name = "reboot", .init = memfaultd_reboot_init},
#endif
#ifdef PLUGIN_SWUPDATE
  {.name = "swupdate", .init = memfaultd_swupdate_init},
#endif
#ifdef PLUGIN_COLLECTD
  {.name = "collectd", .init = memfaultd_collectd_init},
#endif
#ifdef PLUGIN_COREDUMP
  {.name = "coredump", .init = memfaultd_coredump_init, .ipc_name = "CORE"},
#endif
  {NULL, NULL, "", ""}};

static sMemfaultd *s_handle;

#ifndef VERSION
  #define VERSION dev
#endif
#ifndef GITCOMMIT
  #define GITCOMMIT unknown
#endif

#define STRINGIZE(x) #x
#define STRINGIZE_VALUE_OF(x) STRINGIZE(x)

#define NETWORK_FAILURE_FIRST_BACKOFF_SECONDS 60
#define NETWORK_FAILURE_BACKOFF_MULTIPLIER 2

const char memfaultd_sdk_version[] = STRINGIZE_VALUE_OF(VERSION);

/**
 * @brief Displays SDK version information
 *
 */
static void prv_memfaultd_version_info(void) {
  printf("VERSION=%s\n", STRINGIZE_VALUE_OF(VERSION));
  printf("GIT COMMIT=%s\n", STRINGIZE_VALUE_OF(GITCOMMIT));
}

/**
 * @brief Displays usage information
 *
 */
static void prv_memfaultd_usage(void) {
  printf("Usage: memfaultd [OPTION]...\n\n");
  printf("      --config-file <file>       : Configuration file\n");
  printf("      --daemonize                : Daemonize process\n");
  printf("      --enable-data-collection   : Enable data collection, will restart the main "
         "memfaultd service\n");
  printf("      --disable-data-collection  : Disable data collection, will restart the main "
         "memfaultd service\n");
  printf("  -h, --help                     : Display this help and exit\n");
  printf("  -s, --show-settings            : Show settings\n");
  printf("  -v, --version                  : Show version information\n");
}

/**
 * @brief Call the init function of all defined plugins
 *
 * @param handle Main memfaultd handle
 */
static void prv_memfaultd_load_plugins(sMemfaultd *handle) {
  for (unsigned int i = 0; i < MEMFAULT_ARRAY_SIZE(s_plugins); ++i) {
    if (s_plugins[i].init != NULL && !s_plugins[i].init(handle, &s_plugins[i].fns)) {
      fprintf(stderr, "memfaultd:: Failed to initialize %s plugin, destroying.\n",
              s_plugins[i].name);
      s_plugins[i].fns = NULL;
    }
  }
}

/**
 * @brief Call the destroy function of all defined plugins
 */
static void prv_memfaultd_destroy_plugins(void) {
  for (unsigned int i = 0; i < MEMFAULT_ARRAY_SIZE(s_plugins); ++i) {
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
static void prv_memfaultd_enable_collection(sMemfaultd *handle, bool enable) {
  bool current_state = false;
  if (memfaultd_get_boolean(handle, "", "enable_data_collection", &current_state) &&
      current_state == enable) {
    printf("Data collection state already set\n");
    return;
  }
  printf("%s data collection\n", enable ? "Enabling" : "Disabling");
  memfaultd_set_boolean(handle, "", "enable_data_collection", enable);

  if (getuid() == 0 &&
      !memfaultd_utils_restart_service_if_running("memfaultd", "memfaultd.service")) {
    fprintf(stderr, "memfaultd:: Failed to restart memfaultd.\n");
  }
}

/**
 * @brief Signal handler
 *
 * @param sig Signal number
 */
static void prv_memfaultd_sig_handler(int sig) {
  if (sig == SIGUSR1) {
    // Used to service the TX queue
    // The signal has already woken up the main thread from its sleep(), can simply exit here
    return;
  }

  fprintf(stderr, "memfaultd:: Received signal %u, shutting down.\n", sig);
  s_handle->terminate = true;

  // shutdown() the read-side of the socket to abort any in-progress recv() calls
  shutdown(s_handle->ipc_socket_fd, SHUT_RD);
}

/**
 * @brief Looks up the HTTP API path given a eMemfaultdTxDataType.
 * @param type The type to lookup.
 * @return The HTTP API path or NULL if the type is unknown.
 */
static const char *prv_endpoint_for_txdata_type(uint8_t type) {
  switch (type) {
    case kMemfaultdTxDataType_RebootEvent:
      return "/api/v0/events";
    case kMemfaultdTxDataType_CoreUpload:
      return "/api/v0/upload/elf_coredump";
    default:
      fprintf(stderr, "memfaultd:: Unrecognised queue type '%d'\n", type);
      return NULL;
  }
}

/**
 * @brief Process TX queue and transmit messages
 *
 * @param handle Main memfaultd handle
 * @return true Successfully processed the queue, sending all valid entries
 * @return false Failed to process
 */
static bool prv_memfaultd_process_tx_queue(sMemfaultd *handle) {
  bool allowed;
  if (!memfaultd_get_boolean(handle, NULL, "enable_data_collection", &allowed) || !allowed) {
    return true;
  }

  uint32_t queue_entry_size_bytes;
  uint8_t *queue_entry;
  while ((queue_entry = memfaultd_queue_read_head(handle->queue, &queue_entry_size_bytes))) {
    const sMemfaultdTxData *txdata = (const sMemfaultdTxData *)queue_entry;
    const char *endpoint = prv_endpoint_for_txdata_type(txdata->type);
    // FIXME: assuming NUL terminated UTF-8 C string -- need to support binary data too:
    const char *payload = (const char *)txdata->payload;
    if (endpoint == NULL) {
      free(queue_entry);
      memfaultd_queue_complete_read(handle->queue);
      continue;
    }

    eMemfaultdNetworkResult rc = kMemfaultdNetworkResult_ErrorNoRetry;
    switch (txdata->type) {
      case kMemfaultdTxDataType_RebootEvent:
        rc = memfaultd_network_post(handle->network, endpoint, payload, NULL, 0);
        break;
      case kMemfaultdTxDataType_CoreUpload:
        rc = memfaultd_network_file_upload(handle->network, endpoint, payload);
        break;
    }

    free(queue_entry);
    if (rc == kMemfaultdNetworkResult_OK || rc == kMemfaultdNetworkResult_ErrorNoRetry) {
      memfaultd_queue_complete_read(handle->queue);
    } else {
      // Retry-able error
      return false;
    }
  }

  return true;
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
  time_t next_telemetry_poll = 0;
  int override_interval = NETWORK_FAILURE_FIRST_BACKOFF_SECONDS;
  while (!handle->terminate) {
    struct timeval last_wakeup;
    gettimeofday(&last_wakeup, NULL);

    int interval = 1 * 60 * 60;
    memfaultd_get_integer(handle, NULL, "refresh_interval_seconds", &interval);

    if (next_telemetry_poll + interval >= last_wakeup.tv_sec) {
      next_telemetry_poll = last_wakeup.tv_sec + interval;
      // Unimplemented: Perform data collection calls
    }

    if (prv_memfaultd_process_tx_queue(handle)) {
      // Reset override in preparation of next failure
      override_interval = NETWORK_FAILURE_FIRST_BACKOFF_SECONDS;
    } else {
      // call failed, back off up to the entire update interval
      interval = MIN(override_interval, interval);
      override_interval *= NETWORK_FAILURE_BACKOFF_MULTIPLIER;
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

  prv_memfaultd_version_info();
  printf("\n");

  printf("Plugin enabled:\n");
  for (unsigned int i = 0; i < MEMFAULT_ARRAY_SIZE(s_plugins); ++i) {
    if (strlen(s_plugins[i].name) != 0) {
      printf("  %s\n", s_plugins[i].name);
    }
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

static void *prv_ipc_process_thread(void *arg) {
  sMemfaultd *handle = arg;

  if ((handle->ipc_socket_fd = socket(AF_UNIX, SOCK_DGRAM, 0)) == -1) {
    fprintf(stderr, "memfault:: Failed to create listening socket : %s\n", strerror(errno));
    goto cleanup;
  }

  struct sockaddr_un addr = {.sun_family = AF_UNIX};
  strncpy(addr.sun_path, SOCKET_PATH, sizeof(addr.sun_path) - 1);
  if (unlink(SOCKET_PATH) == -1 && errno != ENOENT) {
    fprintf(stderr, "memfault:: Failed to remove IPC socket file '%s' : %s\n", SOCKET_PATH,
            strerror(errno));
    goto cleanup;
  }

  if (bind(handle->ipc_socket_fd, (struct sockaddr *)&addr, sizeof(addr)) == -1) {
    fprintf(stderr, "memfault:: Failed to bind to listener address() : %s\n", strerror(errno));
    goto cleanup;
  }

  while (!handle->terminate) {
    char ctrl_buf[CMSG_SPACE(sizeof(int))] = {'\0'};

    struct iovec iov[1] = {{.iov_base = handle->ipc_rx_buffer, .iov_len = RX_BUFFER_SIZE}};

    struct sockaddr_un src_addr = {0};

    struct msghdr msg = {
      .msg_name = (struct sockaddr *)&src_addr,
      .msg_namelen = sizeof(src_addr),
      .msg_iov = iov,
      .msg_iovlen = 1,
      .msg_control = ctrl_buf,
      .msg_controllen = sizeof(ctrl_buf),
    };

    size_t received_size;
    if ((received_size = recvmsg(handle->ipc_socket_fd, &msg, 0)) <= 0 || msg.msg_iovlen != 1) {
      continue;
    }

    for (unsigned int i = 0; i < MEMFAULT_ARRAY_SIZE(s_plugins); ++i) {
      if (strcmp(s_plugins[i].ipc_name, "") == 0 || !s_plugins[i].fns ||
          !s_plugins[i].fns->plugin_ipc_msg_handler) {
        // Plugin doesn't process IPC messages
        continue;
      }

      if (received_size <= strlen(s_plugins[i].ipc_name) ||
          strcmp(s_plugins[i].ipc_name, msg.msg_iov[0].iov_base) != 0) {
        // Plugin doesn't match IPC signature
        continue;
      }

      if (!s_plugins[i].fns->plugin_ipc_msg_handler(s_plugins[i].fns->handle, handle->ipc_socket_fd,
                                                    &msg, received_size)) {
        fprintf(stderr, "memfault:: '%s' plugin matched IPC message, but failed to process\n",
                s_plugins[i].name);
      }
      break;
    }
  }

cleanup:
  close(handle->ipc_socket_fd);
  if (unlink(SOCKET_PATH) == -1 && errno != ENOENT) {
    fprintf(stderr, "memfault:: Failed to remove IPC socket file '%s' : %s\n", SOCKET_PATH,
            strerror(errno));
  }

  return (void *)NULL;
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
  bool enable_comms = false;
  bool disable_comms = false;
  bool display_config = false;

  s_handle = calloc(sizeof(sMemfaultd), 1);
  const char *config_file = CONFIG_FILE;

  static struct option sMemfaultdLongOptions[] = {
    {"config-file", required_argument, NULL, 'c'},
    {"disable-data-collection", no_argument, NULL, 'd'},
    {"enable-data-collection", no_argument, NULL, 'e'},
    {"help", no_argument, NULL, 'h'},
    {"show-settings", no_argument, NULL, 's'},
    {"version", no_argument, NULL, 'v'},
    {"daemonize", no_argument, NULL, 'Z'},
    {NULL, 0, NULL, 0}};

  int opt;
  while ((opt = getopt_long(argc, argv, "c:dehsvZ", sMemfaultdLongOptions, NULL)) != -1) {
    switch (opt) {
      case 'c':
        config_file = optarg;
        break;
      case 'd':
        disable_comms = true;
        break;
      case 'e':
        enable_comms = true;
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
      case 'Z':
        daemonize = true;
        break;
      default:
        exit(EXIT_FAILURE);
    }
  }

  if (!(s_handle->config = memfaultd_config_init(s_handle, config_file))) {
    fprintf(stderr, "memfaultd:: Failed to create config object, aborting.\n");
    exit(EXIT_FAILURE);
  }

  memfaultd_create_data_dir(s_handle);

  if (enable_comms || disable_comms) {
    if (enable_comms && disable_comms) {
      fprintf(stderr, "memfaultd:: Unable to enable and disable comms simultaneously\n");
      exit(EXIT_FAILURE);
    }
    prv_memfaultd_enable_collection(s_handle, enable_comms);
    exit(EXIT_SUCCESS);
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

  if (!daemonize && prv_memfaultd_check_for_pid_file()) {
    fprintf(stderr, "memfaultd:: memfaultd already running, pidfile: '%s'.\n", PID_FILE);
    exit(EXIT_FAILURE);
  }

  //! Disable coredumping of this process
  prctl(PR_SET_DUMPABLE, 0, 0, 0);

  signal(SIGTERM, prv_memfaultd_sig_handler);
  signal(SIGHUP, prv_memfaultd_sig_handler);
  signal(SIGINT, prv_memfaultd_sig_handler);
  signal(SIGUSR1, prv_memfaultd_sig_handler);

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

  // Ensure startup scroll is pushed to journal
  fflush(stdout);

  if (pthread_create(&s_handle->ipc_thread_id, NULL, prv_ipc_process_thread, s_handle) != 0) {
    fprintf(stderr, "ipc:: Failed to create handler thread\n");
    exit(EXIT_FAILURE);
  }

  prv_memfaultd_process_loop(s_handle);

  pthread_join(s_handle->ipc_thread_id, NULL);

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

char *memfaultd_generate_rw_filename(sMemfaultd *handle, const char *filename) {
  return memfaultd_config_generate_rw_filename(handle->config, filename);
}
