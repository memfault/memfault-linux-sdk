#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! config init and handling definition

#include <memfaultd.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CONFIG_FILE "/etc/memfaultd.conf"
#define CONFIG_KEY_DEV_MODE "enable_dev_mode"
#define CONFIG_KEY_DATA_COLLECTION "enable_data_collection"

typedef struct MemfaultdConfig sMemfaultdConfig;

sMemfaultdConfig *memfaultd_config_init(const char *file);
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
bool memfaultd_config_get_optional_string(sMemfaultdConfig *handle, const char *parent_key,
                                          const char *key, const char **val);
bool memfaultd_config_get_integer(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  int *val);
bool memfaultd_config_get_boolean(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  bool *val);
bool memfaultd_config_get_objects(sMemfaultdConfig *handle, const char *parent_key,
                                  sMemfaultdConfigObject **objects, int *len);
char *memfaultd_config_generate_persisted_filename(sMemfaultdConfig *handle, const char *filename);
char *memfaultd_config_generate_tmp_filename(sMemfaultdConfig *handle, const char *filename);

#ifdef __cplusplus
}
#endif
