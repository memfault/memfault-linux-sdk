//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! config init and handling definition

#ifndef __MEMFAULT_config_H
#define __MEMFAULT_config_H

#include <memfaultd.h>
#include <stdbool.h>

typedef struct MemfaultdConfig sMemfaultdConfig;

sMemfaultdConfig *memfaultd_config_init(sMemfaultd *memfaultd, const char *file);
void memfaultd_config_destroy(sMemfaultdConfig *handle);
void memfaultd_config_dump_config(sMemfaultdConfig *handle, const char *file);

void memfaultd_config_set_string(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                 const char *val);
void memfaultd_config_set_integer(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  const int val);
void memfaultd_config_set_boolean(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  const bool val);

bool memfaultd_config_get_string(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                 const char **val);
bool memfaultd_config_get_integer(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  int *val);
bool memfaultd_config_get_boolean(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  bool *val);
bool memfaultd_config_get_objects(sMemfaultdConfig *handle, const char *parent_key,
                                  sMemfaultdConfigObject **objects, int *len);
char *memfaultd_config_generate_rw_filename(sMemfaultdConfig *handle, const char *filename);

#endif
