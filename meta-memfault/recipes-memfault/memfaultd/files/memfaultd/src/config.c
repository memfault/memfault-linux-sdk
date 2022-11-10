//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! config init and handling implementation

#include "config.h"

#include <errno.h>
#include <fcntl.h>
#include <json-c/json.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "builtin_conf.h"
#include "memfaultd.h"

struct MemfaultdConfig {
  sMemfaultd *memfaultd;
  json_object *base;
  json_object *runtime;
};

/**
 * @brief Write runtime tree to file
 *
 * @param handle
 */
static void prv_config_write_config(sMemfaultdConfig *handle) {
  char *runtime_path = memfaultd_generate_rw_filename(handle->memfaultd, "runtime.conf");
  if (!runtime_path) {
    return;
  }

  if (json_object_to_file_ext(runtime_path, handle->runtime,
                              JSON_C_TO_STRING_SPACED | JSON_C_TO_STRING_PRETTY) == -1) {
    fprintf(stderr, "config:: Failed to update runtime config file '%s'.\n", runtime_path);
  }
  free(runtime_path);
}

/**
 * @brief
 *
 * @param object Source object to merge from
 * @param objects Array of objects to merge into
 * @param len Length or array returned
 */
static void prv_config_merge_object_into_array(json_object *object,
                                               sMemfaultdConfigObject **objects, int *len) {
  if (!object) {
    return;
  }

  json_object_object_foreach(object, key, value) {
    int i = 0;
    bool matched = false;
    for (i = 0; i < (*len); ++i) {
      if (strcmp((*objects)[i].key, key) == 0) {
        matched = true;
        break;
      }
    }
    if (!matched) {
      ++(*len);
      *objects = realloc(*objects, sizeof(sMemfaultdConfigObject) * (*len));
      (*objects)[i].key = key;
    }

    switch (json_object_get_type(value)) {
      case json_type_boolean:
        (*objects)[i].type = kMemfaultdConfigTypeBoolean;
        (*objects)[i].value.b = json_object_get_boolean(value);
        break;
      case json_type_int:
        (*objects)[i].type = kMemfaultdConfigTypeInteger;
        (*objects)[i].value.d = json_object_get_int(value);
        break;
      case json_type_string:
        (*objects)[i].type = kMemfaultdConfigTypeString;
        (*objects)[i].value.s = json_object_get_string(value);
        break;
      case json_type_object:
        (*objects)[i].type = kMemfaultdConfigTypeObject;
        break;
      default:
        (*objects)[i].type = kMemfaultdConfigTypeUnknown;
        break;
    }
  }
}

/**
 * @brief Find object by parent&key in given tree
 *
 * @param tree Base object to find sub-object in
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @return json_object* Found object
 */
static json_object *prv_config_find_object(json_object *tree, const char *parent_key,
                                           const char *key) {
  if (!tree) {
    return NULL;
  }
  json_object *object = NULL;
  if (parent_key && strlen(parent_key) != 0) {
    if (!json_object_object_get_ex(tree, parent_key, &object) ||
        json_object_get_type(object) != json_type_object) {
      return NULL;
    }
  } else {
    object = tree;
  }
  if (object) {
    if (key && strlen(key) != 0) {
      if (json_object_object_get_ex(object, key, &object)) {
        return object;
      }
    } else {
      return object;
    }
  }
  return NULL;
}

/**
 * @brief Set object into parent&key in runtime tree
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 */
static void prv_config_set_object(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  json_object *val) {
  json_object *object;
  if (parent_key && strlen(parent_key)) {
    if (!json_object_object_get_ex(handle->runtime, parent_key, &object) ||
        json_object_get_type(object) != json_type_object) {
      object = json_object_new_object();
      json_object_object_add(handle->runtime, parent_key, object);
    }
  } else {
    object = handle->runtime;
  }
  json_object_object_add(object, key, val);

  prv_config_write_config(handle);
}

/**
 * @brief Deep copy of Source object into Destination
 *
 * @param a Destination
 * @param b Source
 */
static void prv_config_merge_objects(json_object *a, json_object *b) {
  json_object_object_foreach(b, key, val) {
    if (json_object_get_type(val) == json_type_object) {
      json_object *tmp;
      if (json_object_object_get_ex(a, key, &tmp) &&
          json_object_get_type(tmp) == json_type_object) {
        prv_config_merge_objects(tmp, val);
      } else {
        json_object_object_add(a, key, val);
        json_object_get(val);
      }
    } else {
      json_object_object_add(a, key, val);
      json_object_get(val);
    }
  }
}

char *memfaultd_config_generate_rw_filename(sMemfaultdConfig *handle, const char *filename) {
  const char *data_dir;
  char *file = NULL;
  if (memfaultd_config_get_string(handle, "", "data_dir", &data_dir) && strlen(data_dir) != 0) {
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

/**
 * @brief Initialise the config object
 *
 * @param file Master config filename
 * @return memfaultd_config_h* config object
 */
sMemfaultdConfig *memfaultd_config_init(sMemfaultd *memfaultd, const char *file) {
  sMemfaultdConfig *handle = calloc(sizeof(sMemfaultdConfig), 1);

  handle->memfaultd = memfaultd;

  handle->base = json_tokener_parse((char *)builtin_conf);

  int fd;
  if ((fd = open(file, O_RDONLY)) == -1) {
    if (errno == ENOENT) {
      fprintf(stderr, "config:: Configuration file '%s' not found.\n", file);
    } else {
      fprintf(stderr, "config:: Unable to open configuration file '%s', %s.\n", file,
              strerror(errno));
    }
  } else {
    json_object *object = json_object_from_fd(fd);
    if (!object) {
      fprintf(stderr, "config:: Unable to parse configuration file  '%s': %s\n",
              file, json_util_get_last_err());
    } else {
      prv_config_merge_objects(handle->base, object);
      json_object_put(object);
    }
    close(fd);
  }

  char *runtime_path = memfaultd_config_generate_rw_filename(handle, "runtime.conf");

  if (!runtime_path) {
    // No runtime config, warn but continue.
    fprintf(stderr, "config:: No runtime_config defined, settings will not persist.\n");
    handle->runtime = json_object_new_object();
  } else {
    if ((fd = open(runtime_path, O_RDONLY)) == -1) {
      if (errno != ENOENT) {
        // Missing file is not an error
        fprintf(stderr, "config:: Unable to open configuration file '%s', %s.\n", runtime_path,
                strerror(errno));
      }
      handle->runtime = json_object_new_object();
    } else {
      if (!(handle->runtime = json_object_from_fd(fd))) {
        fprintf(stderr, "config:: Unable to parse configuration file  '%s'\n", runtime_path);
        handle->runtime = json_object_new_object();
      }
      close(fd);
    }
    free(runtime_path);
  }

  return handle;
}

/**
 * @brief Destroy the config object
 *
 * @param handle config object
 */
void memfaultd_config_destroy(sMemfaultdConfig *handle) {
  if (handle) {
    if (handle->base) {
      json_object_put(handle->base);
    }
    if (handle->runtime) {
      json_object_put(handle->runtime);
    }
    free(handle);
  }
}

/**
 * @brief Dump both configuration trees to stdout
 *
 * @param handle config object
 */
void memfaultd_config_dump_config(sMemfaultdConfig *handle, const char *file) {
  printf("Base configuration (%s):\n", file);
  printf("  %s\n\n", json_object_to_json_string(handle->base));
  printf("Runtime configuration:\n");
  printf("  %s\n\n", json_object_to_json_string(handle->runtime));
}

/**
 * @brief Set string in config object
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @param val Value to set
 */
void memfaultd_config_set_string(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                 const char *val) {
  prv_config_set_object(handle, parent_key, key, json_object_new_string(val));
}

/**
 * @brief Set integer in config object
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @param val Value to set
 */
void memfaultd_config_set_integer(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  int val) {
  prv_config_set_object(handle, parent_key, key, json_object_new_int(val));
}

/**
 * @brief Set boolean flag in config object
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @param val Value to set
 */
void memfaultd_config_set_boolean(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  bool val) {
  prv_config_set_object(handle, parent_key, key, json_object_new_boolean(val));
}

/**
 * @brief Get string from config object
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @param val Value returned
 * @return true Successfully added string to config
 * @return false Failed to add string
 */
bool memfaultd_config_get_string(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                 const char **val) {
  json_object *object;

  if (!(object = prv_config_find_object(handle->runtime, parent_key, key)) &&
      !(object = prv_config_find_object(handle->base, parent_key, key))) {
    fprintf(stderr, "config:: Failed to find config object %s:%s \n", parent_key ? parent_key : "",
            key ? key : "");
    return false;
  }

  if (json_object_get_type(object) != json_type_string) {
    fprintf(stderr, "config:: Object is not of type %s %s:%s \n", "string",
            parent_key ? parent_key : "", key ? key : "");
    return false;
  }

  *val = json_object_get_string(object);
  return true;
}

/**
 * @brief Get integer from config object
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @param val Value returned
 * @return true Successfully added integer to config
 * @return false Failed to add integer
 */
bool memfaultd_config_get_integer(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  int *val) {
  json_object *object;

  if (!(object = prv_config_find_object(handle->runtime, parent_key, key)) &&
      !(object = prv_config_find_object(handle->base, parent_key, key))) {
    fprintf(stderr, "config:: Failed to find config object %s:%s \n", parent_key ? parent_key : "",
            key ? key : "");
    return false;
  }

  if (json_object_get_type(object) != json_type_int) {
    fprintf(stderr, "config:: Object is not of type %s %s:%s \n", "int",
            parent_key ? parent_key : "", key ? key : "");
    return false;
  }

  *val = json_object_get_int(object);
  return true;
}

/**
 * @brief Get boolean flag from config object
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param key Key name to set
 * @param val Value returned
 * @return true Successfully added boolean to config
 * @return false Failed to add boolean
 */
bool memfaultd_config_get_boolean(sMemfaultdConfig *handle, const char *parent_key, const char *key,
                                  bool *val) {
  json_object *object;

  if (!(object = prv_config_find_object(handle->runtime, parent_key, key)) &&
      !(object = prv_config_find_object(handle->base, parent_key, key))) {
    fprintf(stderr, "config:: Failed to find config object %s:%s \n", parent_key ? parent_key : "",
            key ? key : "");
    return false;
  }

  if (json_object_get_type(object) != json_type_boolean) {
    fprintf(stderr, "config:: Object is not of type %s %s:%s \n", "boolean",
            parent_key ? parent_key : "", key ? key : "");
    return false;
  }

  *val = json_object_get_boolean(object);
  return true;
}

/**
 * @brief Get complete object structure
 *
 * @param handle config object
 * @param parent_key Parent key name, NULL for root object
 * @param objects Array of objects returned
 * @param len Length of array returned
 * @return true Successfully retrieved object structure
 * @return false Failed to retrieved structure
 */
bool memfaultd_config_get_objects(sMemfaultdConfig *handle, const char *parent_key,
                                  sMemfaultdConfigObject **objects, int *len) {
  *len = 0;
  *objects = NULL;

  json_object *object = NULL;
  if ((object = prv_config_find_object(handle->base, parent_key, "")) &&
      json_object_get_type(object) == json_type_object) {
    prv_config_merge_object_into_array(object, objects, len);
  }

  if ((object = prv_config_find_object(handle->runtime, parent_key, "")) &&
      json_object_get_type(object) == json_type_object) {
    prv_config_merge_object_into_array(object, objects, len);
  }

  return true;
}
