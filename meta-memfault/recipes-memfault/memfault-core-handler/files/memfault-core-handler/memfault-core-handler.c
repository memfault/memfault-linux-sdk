//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Memfault CORE-ELF handler

#include <errno.h>
#include <signal.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>

#define SOCKET_PATH "/tmp/memfault-ipc.sock"
#define TX_BUFFER_SIZE 1024

static int completion_timeout = 60;
static int tx_retry_count = 10;

static int fd = -1;
char resp_socket[] = "/tmp/memfault-core-handler-XXXXXX.sock";

static void prv_sig_handler(int sig) {
  fprintf(stderr, "Unexpected signal %d\n", sig);
  if (fd != -1) {
    shutdown(fd, SHUT_RDWR);
  }
  if (unlink(resp_socket) == -1 && errno != ENOENT) {
    fprintf(stderr, "Failed to remove tmp socket file '%s' : %s\n", resp_socket,
            strerror(errno));
  }
}

static void prv_usage(const char *cmd) {
  printf("Usage: %s [-r tx_retry_count] [-t completion_timeout] ARGS ...\n",
         cmd);
}

int main(int argc, char *argv[]) {
  char buf[TX_BUFFER_SIZE] = {'\0'};
  int ret = EXIT_FAILURE;

  int opt;
  while ((opt = getopt(argc, argv, "hr:t:")) != -1) {
    switch (opt) {
    case 'r':
      tx_retry_count = strtol(optarg, NULL, 0);
      break;
    case 't':
      completion_timeout = strtol(optarg, NULL, 0);
      break;
    case 'h':
    default:
      prv_usage(argv[0]);
      exit(EXIT_FAILURE);
    }
  }
  int first_forwarded_arg = optind;

  //! Disable coredumping of this process
  prctl(PR_SET_DUMPABLE, 0, 0, 0);

  signal(SIGTERM, prv_sig_handler);
  signal(SIGHUP, prv_sig_handler);
  signal(SIGINT, prv_sig_handler);
  signal(SIGSEGV, prv_sig_handler);

  if ((fd = socket(AF_UNIX, SOCK_DGRAM, 0)) == -1) {
    fprintf(stderr, "Failed to create socket() : %s\n", strerror(errno));
    goto cleanup;
  }

  int tmp_socket;
  if ((tmp_socket = mkstemps(resp_socket, 5)) == -1) {
    fprintf(stderr, "Failed to create response socket file\n");
    goto cleanup;
  }
  close(tmp_socket);
  if (unlink(resp_socket) == -1 && errno != ENOENT) {
    fprintf(stderr, "Failed to remove tmp socket file '%s' : %s\n", resp_socket,
            strerror(errno));
    goto cleanup;
  }

  struct sockaddr_un resp_addr = {.sun_family = AF_UNIX};
  strncpy(resp_addr.sun_path, resp_socket, sizeof(resp_addr.sun_path) - 1);

  if (bind(fd, (struct sockaddr *)&resp_addr, sizeof(resp_addr)) == -1) {
    fprintf(stderr, "Failed to bind() to response socket : %s\n",
            strerror(errno));
    goto cleanup;
  }

  struct sockaddr_un server_addr = {.sun_family = AF_UNIX};
  strncpy(server_addr.sun_path, SOCKET_PATH, sizeof(server_addr.sun_path) - 1);

  int offset = 0;
  strncpy(&buf[offset], "CORE", TX_BUFFER_SIZE - offset - 1);
  offset += (strlen(&buf[offset]) + 1);
  strncpy(&buf[offset], "ELF", TX_BUFFER_SIZE - offset - 1);
  offset += (strlen(&buf[offset]) + 1);
  for (int i = first_forwarded_arg; i < argc; ++i) {
    strncpy(&buf[offset], argv[i], TX_BUFFER_SIZE - offset - 1);
    offset += (strlen(&buf[offset]) + 1);
  }

  struct iovec iov[1] = {{.iov_base = buf, .iov_len = offset}};

  char ctrl_buf[CMSG_LEN(sizeof(int))];
  struct msghdr msg = {.msg_name = &server_addr,
                       .msg_namelen = sizeof(server_addr),
                       .msg_iov = iov,
                       .msg_iovlen = 1,
                       .msg_control = ctrl_buf,
                       .msg_controllen = CMSG_LEN(sizeof(int))};

  struct cmsghdr *cmsg = CMSG_FIRSTHDR(&msg);
  *cmsg = (struct cmsghdr){.cmsg_level = SOL_SOCKET,
                           .cmsg_type = SCM_RIGHTS,
                           .cmsg_len = sizeof(ctrl_buf)};
  *(int *)CMSG_DATA(cmsg) = STDIN_FILENO;

  int i = 0;
  while (true) {
    if (sendmsg(fd, &msg, 0) == -1) {
      // Wait up to tx_retry_count times for memfaultd to restart
      if (++i > tx_retry_count) {
        fprintf(stderr, "Failed to sendmsg() to memfaultd : %s\n",
                strerror(errno));
        goto cleanup;
      }
      sleep(1);
    } else {
      break;
    }
  }

  const struct timeval tv = {.tv_sec = completion_timeout};
  if (setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, (const char *)&tv, sizeof tv) ==
      -1) {
    fprintf(stderr,
            "Failed to setsockopt() receive timeout, process will not "
            "timeout if memfaultd crashes : %s\n",
            strerror(errno));
  }

  if (recv(fd, buf, TX_BUFFER_SIZE, 0) != sizeof(int)) {
    fprintf(stderr, "Failed to recv() valid exit-code from memfaultd : %s\n",
            strerror(errno));
    goto cleanup;
  }
  ret = (int)*buf;
  if (ret != EXIT_SUCCESS) {
    fprintf(stderr, "memfaultd returned unexpected exit-code %d\n", ret);
  }

cleanup:
  if (fd != -1) {
    close(fd);
  }
  if (unlink(resp_socket) == -1 && errno != ENOENT) {
    fprintf(stderr, "Failed to remove tmp socket file '%s' : %s\n", resp_socket,
            strerror(errno));
  }

  return ret;
}
