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
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/param.h>
#if __linux__
  #include <sys/prctl.h>
#endif
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#include "memfault/core/math.h"
#include "memfault/util/config.h"
#include "memfault/util/device_settings.h"
#include "memfault/util/disk.h"
#include "memfault/util/dump_settings.h"
#include "memfault/util/ipc.h"
#include "memfault/util/pid.h"
#include "memfault/util/plugins.h"
#include "memfault/util/runtime_config.h"
#include "memfault/util/string.h"
#include "memfault/util/systemd.h"
#include "memfault/util/version.h"
#include "queue.h"

#define RX_BUFFER_SIZE 1024
#define PID_FILE "/var/run/memfaultd.pid"

struct Memfaultd {
  sMemfaultdQueue *queue;
  sMemfaultdConfig *config;
  const char *config_file;
  sMemfaultdDeviceSettings *settings;
  bool terminate;
  bool dev_mode;
  pthread_t ipc_thread_id;
  int ipc_socket_fd;
  char ipc_rx_buffer[RX_BUFFER_SIZE];
};

static sMemfaultd *s_handle;

extern bool memfaultd_rust_process_loop(const char *, sMemfaultdQueue *);

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
  printf("      --enable-dev-mode          : Enable developer mode (restarts memfaultd)\n");
  printf("      --disable-dev-mode         : Disable developer mode (restarts memfaultd)\n");
  printf("  -h, --help                     : Display this help and exit\n");
  printf("  -s, --show-settings            : Show settings\n");
  printf("  -v, --version                  : Show version information\n");
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

#if __linux__
  // Use noclose=1 so logs written to stdout/stderr can be viewed using journalctl
  if (daemon(0, 1) == -1) {
    fprintf(stderr, "memfaultd:: Failed to daemonize, aborting.\n");
    return false;
  }
#else
  // daemon() is deprecated on macOS
  fprintf(stderr, "Not linux - not daemonizing.");
#endif

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

  struct sockaddr_un addr = {.sun_family = AF_UNIX};
  strncpy(addr.sun_path, MEMFAULTD_IPC_SOCKET_PATH, sizeof(addr.sun_path) - 1);
  if (unlink(MEMFAULTD_IPC_SOCKET_PATH) == -1 && errno != ENOENT) {
    fprintf(stderr, "memfault:: Failed to remove IPC socket file '%s' : %s\n",
            MEMFAULTD_IPC_SOCKET_PATH, strerror(errno));
    goto cleanup;
  }

  if (bind(handle->ipc_socket_fd, (struct sockaddr *)&addr, sizeof(addr)) == -1) {
    fprintf(stderr, "memfault:: Failed to bind to listener address() : %s\n", strerror(errno));
    goto cleanup;
  }

  // This loop is interrupted by the main thread calling shutdown() on the socket.
  while (1) {
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
      if (received_size == 0) {
        // recvmsg will return 0 when (and only when) the socket has been shutdown.
        break;
      }
      continue;
    }

    if (!memfaultd_plugins_process_ipc(&msg, received_size)) {
      fprintf(stderr, "memfaultd:: Failed to process IPC message (no plugin).\n");
    }
  }

cleanup:
  close(handle->ipc_socket_fd);
  if (unlink(MEMFAULTD_IPC_SOCKET_PATH) == -1 && errno != ENOENT) {
    fprintf(stderr, "memfault:: Failed to remove IPC socket file '%s' : %s\n",
            MEMFAULTD_IPC_SOCKET_PATH, strerror(errno));
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
int memfaultd_main(int argc, char *argv[]) {
  bool daemonize = false;
  bool enable_comms = false;
  bool disable_comms = false;
  bool display_config = false;
  bool enable_devmode = false;
  bool disable_devmode = false;

  s_handle = calloc(sizeof(sMemfaultd), 1);
  s_handle->config_file = CONFIG_FILE;

  static struct option sMemfaultdLongOptions[] = {
    {"config-file", required_argument, NULL, 'c'},
    {"disable-data-collection", no_argument, NULL, 'd'},
    {"disable-dev-mode", no_argument, NULL, 'm'},
    {"enable-data-collection", no_argument, NULL, 'e'},
    {"enable-dev-mode", no_argument, NULL, 'M'},
    {"help", no_argument, NULL, 'h'},
    {"show-settings", no_argument, NULL, 's'},
    {"version", no_argument, NULL, 'v'},
    {"daemonize", no_argument, NULL, 'Z'},
    {NULL, 0, NULL, 0}};

  int opt;
  while ((opt = getopt_long(argc, argv, "c:dehsvZ", sMemfaultdLongOptions, NULL)) != -1) {
    switch (opt) {
      case 'c':
        s_handle->config_file = optarg;
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
        memfault_version_print_info();
        exit(EXIT_SUCCESS);
      case 'Z':
        daemonize = true;
        break;
      case 'M':
        enable_devmode = true;
        break;
      case 'm':
        disable_devmode = true;
        break;
      default:
        exit(EXIT_FAILURE);
    }
  }

  if (!(s_handle->config = memfaultd_config_init(s_handle->config_file))) {
    fprintf(stderr, "memfaultd:: Failed to create config object, aborting.\n");
    exit(EXIT_FAILURE);
  }

  memfaultd_create_data_dir(s_handle);

  if (enable_comms || disable_comms) {
    if (enable_comms && disable_comms) {
      fprintf(stderr, "memfaultd:: Unable to enable and disable comms simultaneously\n");
      exit(EXIT_FAILURE);
    }
    exit(memfault_set_runtime_bool_and_reload(s_handle->config, CONFIG_KEY_DATA_COLLECTION,
                                              "data collection", enable_comms));
  }

  if (enable_devmode || disable_devmode) {
    if (enable_devmode && disable_devmode) {
      fprintf(stderr, "memfaultd:: Unable to enable and disable dev-mode simultaneously\n");
      exit(EXIT_FAILURE);
    }
    exit(memfault_set_runtime_bool_and_reload(s_handle->config, CONFIG_KEY_DEV_MODE,
                                              "developer mode", enable_devmode));
  }

  if (!(s_handle->settings = memfaultd_device_settings_init())) {
    fprintf(stderr, "memfaultd:: Failed to load all required device settings, aborting.\n");
    exit(EXIT_FAILURE);
  }

  memfaultd_dump_settings(s_handle->settings, s_handle->config, s_handle->config_file);
  if (display_config) {
    /* Already reported above, just exit */
    exit(EXIT_SUCCESS);
  }

  if (!daemonize && memfaultd_check_for_pid_file()) {
    fprintf(stderr, "memfaultd:: memfaultd already running, pidfile: '%s'.\n", PID_FILE);
    exit(EXIT_FAILURE);
  }

  int queue_size = 0;
  memfaultd_get_integer(s_handle, NULL, "queue_size_kib", &queue_size);

  char *queue_file = memfaultd_generate_rw_filename(s_handle, "queue");
  if (!(s_handle->queue = memfaultd_queue_init(queue_file, queue_size * 1024))) {
    fprintf(stderr, "memfaultd:: Failed to create queue object, aborting.\n");
    exit(EXIT_FAILURE);
  }
  free(queue_file);

  bool allowed;
  if (!memfaultd_get_boolean(s_handle, NULL, "enable_data_collection", &allowed) || !allowed) {
    memfaultd_queue_reset(s_handle->queue);
  }

  if (memfaultd_get_boolean(s_handle, NULL, CONFIG_KEY_DEV_MODE, &s_handle->dev_mode) &&
      s_handle->dev_mode == true) {
    fprintf(stderr, "memfaultd:: Starting with developer mode enabled\n");
  }

  memfaultd_load_plugins(s_handle);

  if (daemonize && !prv_memfaultd_daemonize_process()) {
    exit(EXIT_FAILURE);
  }

  // Ensure startup scroll is pushed to journal
  fflush(stdout);

  // Create IPC socket before creating the thread so we can shut it down.
  if ((s_handle->ipc_socket_fd = socket(AF_UNIX, SOCK_DGRAM, 0)) == -1) {
    fprintf(stderr, "memfault:: Failed to create listening socket : %s\n", strerror(errno));
    exit(EXIT_FAILURE);
  }

  if (pthread_create(&s_handle->ipc_thread_id, NULL, prv_ipc_process_thread, s_handle) != 0) {
    fprintf(stderr, "ipc:: Failed to create handler thread\n");
    exit(EXIT_FAILURE);
  }

  // Run the main loop in Rust.
  // This will register a signal handler and stop when SIGINT/SIGTERM is received.
  const bool success = memfaultd_rust_process_loop(s_handle->config_file, s_handle->queue);

  // shutdown() the read-side of the socket to abort any in-progress recv()
  // calls. This will terminate the ipc_thread.
  // This does not work on BSD ("Socket is not connected") so we do
  // not wait for the thread if we get an error.
  if (shutdown(s_handle->ipc_socket_fd, SHUT_RD) == 0) {
    pthread_join(s_handle->ipc_thread_id, NULL);
  }

  memfaultd_destroy_plugins();

  memfaultd_queue_destroy(s_handle->queue);
  memfaultd_config_destroy(s_handle->config);
  memfaultd_device_settings_destroy(s_handle->settings);
  free(s_handle);

  if (daemonize) {
    unlink(PID_FILE);
  }
  return success ? 0 : -1;
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

bool memfaultd_is_dev_mode(sMemfaultd *handle) { return handle->dev_mode; }

const char *memfaultd_get_config_file(sMemfaultd *handle) { return handle->config_file; }
