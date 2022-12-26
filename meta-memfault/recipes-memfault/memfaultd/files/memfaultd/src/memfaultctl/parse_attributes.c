//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Parse attributes from the command line into a JSON object.

#include "parse_attributes.h"

#include <assert.h>
#include <json-c/json.h>
#include <string.h>

static json_object *prv_parse_attribute_value(const char *value) {
  json_object *obj = json_tokener_parse(value);

  if (!obj) {
    return json_object_new_string(value);
  }

  // Do not allow values that are arrays or objects
  if (json_object_get_type(obj) == json_type_array ||
      json_object_get_type(obj) == json_type_object) {
    json_object_put(obj);
    return json_object_new_string(value);
  }

  return obj;
}

bool memfaultd_parse_attributes(const char **argv, int argc, json_object **ret_json) {
  json_object *json = json_object_new_array();
  assert(json != NULL);

  bool parse_error = false;
  for (int i = 0; i < argc && parse_error == false; i++) {
    char *kvp = strdup(argv[i]);
    assert(kvp != NULL);
    char *key = kvp;
    char *value = strchr(kvp, '=');
    if (value == NULL || key == value) {
      parse_error = true;
      goto cleanup;
    }
    // Replace the '=' sign with a \0 to terminate the key string
    *value = '\0';
    // Value starts at the next character
    value++;

    json_object *key_value_pair = json_object_new_object();
    assert(key_value_pair != NULL);
    json_object *string_key = json_object_new_string(key);
    assert(string_key != NULL);
    assert(json_object_object_add(key_value_pair, "string_key", string_key) == 0);

    json_object *attribute_value = prv_parse_attribute_value(value);
    if (attribute_value == NULL) {
      parse_error = true;
      goto cleanup;
    }

    assert(json_object_object_add(key_value_pair, "value", attribute_value) == 0);
    assert(json_object_array_add(json, key_value_pair) == 0);

  cleanup:
    free(kvp);
  }

  if (parse_error || json_object_array_length(json) < 1) {
    json_object_put(json);
    *ret_json = NULL;
    return false;
  }

  *ret_json = json;
  return true;
}
