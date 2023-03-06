//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Simplified definitions for libubootenv. Just enough to get the tests to compile.
//! We only use this file when the system does not have the headers.
//! Implementations will be mocked by the tests.

struct uboot_ctx {
  int _empty;
};
struct uboot_env_device {
  int _empty;
};

int libuboot_initialize(struct uboot_ctx **out, struct uboot_env_device *envdevs);
int libuboot_read_config(struct uboot_ctx *ctx, const char *config);
int libuboot_open(struct uboot_ctx *ctx);
char *libuboot_get_env(struct uboot_ctx *ctx, const char *varname);

void libuboot_close(struct uboot_ctx *ctx);
void libuboot_exit(struct uboot_ctx *ctx);
