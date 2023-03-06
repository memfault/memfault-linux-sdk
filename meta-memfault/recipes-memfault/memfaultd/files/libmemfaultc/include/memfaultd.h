//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd plugin API definition

#ifndef __MEMFAULT_H
#define __MEMFAULT_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/socket.h>

typedef struct Memfaultd sMemfaultd;
typedef struct MemfaultdPlugin sMemfaultdPlugin;

typedef bool (*memfaultd_plugin_reload)(sMemfaultdPlugin *plugin);
typedef void (*memfaultd_plugin_destroy)(sMemfaultdPlugin *plugin);
typedef bool (*memfaultd_plugin_ipc_msg_handler)(sMemfaultdPlugin *handle, struct msghdr *msg,
                                                 size_t received_size);

typedef enum {
  kMemfaultdConfigTypeUnknown,
  kMemfaultdConfigTypeBoolean,
  kMemfaultdConfigTypeInteger,
  kMemfaultdConfigTypeString,
  kMemfaultdConfigTypeObject
} eMemfaultdConfigType;

typedef struct {
  sMemfaultdPlugin *handle;
  memfaultd_plugin_reload plugin_reload;
  memfaultd_plugin_destroy plugin_destroy;
  memfaultd_plugin_ipc_msg_handler plugin_ipc_msg_handler;
} sMemfaultdPluginCallbackFns;

typedef struct {
  char *device_id;
  char *hardware_version;
} sMemfaultdDeviceSettings;

typedef struct {
  const char *key;
  eMemfaultdConfigType type;
  union {
    bool b;
    int d;
    const char *s;
  } value;
} sMemfaultdConfigObject;

typedef bool (*memfaultd_plugin_init)(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);

typedef enum {
  kMemfaultdTxDataType_RebootEvent = 'R',
  kMemfaultdTxDataType_Attributes = 'A',
} eMemfaultdTxDataType;

typedef struct __attribute__((__packed__)) MemfaultdTxData {
  uint8_t type;  // eMemfaultdTxDataType
  uint8_t payload[];
} sMemfaultdTxData;

typedef struct __attribute__((__packed__)) MemfaultdTxDataAttributes {
  uint8_t type;  // eMemfaultdTxDataType
  time_t timestamp;
  char json[];
} sMemfaultdTxDataAttributes;

int memfaultd_main(int argc, char *argv[]);

bool memfaultd_txdata(sMemfaultd *memfaultd, const sMemfaultdTxData *data, uint32_t payload_size);

bool memfaultd_get_boolean(sMemfaultd *memfaultd, const char *parent_key, const char *key,
                           bool *val);
bool memfaultd_get_integer(sMemfaultd *memfaultd, const char *parent_key, const char *key,
                           int *val);
bool memfaultd_get_string(sMemfaultd *memfaultd, const char *parent_key, const char *key,
                          const char **val);
bool memfaultd_get_objects(sMemfaultd *memfaultd, const char *parent_key,
                           sMemfaultdConfigObject **objects, int *len);

const sMemfaultdDeviceSettings *memfaultd_get_device_settings(sMemfaultd *memfaultd);

char *memfaultd_generate_rw_filename(sMemfaultd *memfaultd, const char *filename);

bool memfaultd_is_dev_mode(sMemfaultd *memfaultd);

const char *memfaultd_get_config_file(sMemfaultd *handle);

#ifdef __cplusplus
}
#endif

#endif
