//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Simplified definitions for libsystemd. Just enough to get the tests to compile.
//! We only use this file when the system does not have the headers.
//! Implementations will be mocked by the tests.

#pragma once

#include <stddef.h>

typedef struct {
  int _empty;
} sd_bus;
typedef struct {
  char *name;
} sd_bus_error;

extern sd_bus_error SD_BUS_ERROR_NULL;

int sd_bus_default_system(sd_bus **bus);
int sd_bus_get_property_string(sd_bus *bus, const char *destination, const char *path,
                               const char *interface, const char *member, sd_bus_error *ret_error,
                               char **ret);
void sd_bus_error_free(sd_bus_error *e);
sd_bus *sd_bus_unref(sd_bus *bus);
