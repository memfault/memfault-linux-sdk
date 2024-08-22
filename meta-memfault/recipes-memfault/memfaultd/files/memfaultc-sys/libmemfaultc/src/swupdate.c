//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Memfault SWUpdate config file generation

#include <libconfig.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DEFAULT_SURICATTA_TENANT "default"

#define HAWKBIT_PATH "/api/v0/hawkbit"

typedef struct {
  char *base_url;
  char *software_version;
  char *software_type;
  char *hardware_version;
  char *device_id;
  char *project_key;

  char *input_file;
  char *output_file;
} sMemfaultdSwupdateConfig;

/**
 * @brief Add 'global' section to config
 *
 * @param config config object to build into
 * @return true Successfully added global options to config
 * @return false Failed to add
 */
static bool prv_swupdate_add_globals(config_t *config) {
  if (!config_lookup(config, "globals")) {
    if (!config_setting_add(config_root_setting(config), "globals", CONFIG_TYPE_GROUP)) {
      fprintf(stderr, "swupdate:: Failed to add globals setting group\n");
      return false;
    }
  }
  return true;
}

/**
 * @brief Add 'suricatta' section to config
 *
 * @param handle swupdate config handle
 * @param config config object to build into
 * @return true Successfully added suricatta options to config
 * @return false Failed to add
 */
static bool prv_swupdate_add_suricatta(sMemfaultdSwupdateConfig *handle, config_t *config) {
  config_setting_t *suricatta = config_lookup(config, "suricatta");
  if (!suricatta) {
    if (!(suricatta =
            config_setting_add(config_root_setting(config), "suricatta", CONFIG_TYPE_GROUP))) {
      fprintf(stderr, "swupdate:: Failed to add suricatta group\n");
      return false;
    }
  }

  char *url = malloc(strlen(HAWKBIT_PATH) + strlen(handle->base_url) + 1);
  strcpy(url, handle->base_url);
  strcat(url, HAWKBIT_PATH);

  config_setting_t *element;
  config_setting_remove(suricatta, "url");
  if (!(element = config_setting_add(suricatta, "url", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, url)) {
    fprintf(stderr, "swupdate:: Failed to add suricatta:url\n");
    free(url);
    return false;
  }

  free(url);

  config_setting_remove(suricatta, "id");
  if (!(element = config_setting_add(suricatta, "id", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, handle->device_id)) {
    fprintf(stderr, "swupdate:: Failed to add suricatta:id\n");
    return false;
  }

  config_setting_remove(suricatta, "tenant");
  if (!(element = config_setting_add(suricatta, "tenant", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, DEFAULT_SURICATTA_TENANT)) {
    fprintf(stderr, "swupdate:: Failed to add suricatta:tenant\n");
    return false;
  }

  config_setting_remove(suricatta, "gatewaytoken");
  if (!(element = config_setting_add(suricatta, "gatewaytoken", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, handle->project_key)) {
    fprintf(stderr, "swupdate:: Failed to add suricatta:id\n");
    return false;
  }

  return true;
}

/**
 * @brief Add 'identify' section to config
 *
 * @param handle swupdate config handle
 * @param config config object to build into
 * @return true Successfully added identify options to config
 * @return false Failed to add
 */
static bool prv_swupdate_add_identify(sMemfaultdSwupdateConfig *handle, config_t *config) {
  config_setting_t *identify;

  config_setting_remove(config_root_setting(config), "identify");
  if (!(identify = config_setting_add(config_root_setting(config), "identify", CONFIG_TYPE_LIST))) {
    fprintf(stderr, "swupdate:: Failed to add identify list\n");
    return false;
  }

  config_setting_t *setting;
  config_setting_t *element;
  if (!(setting = config_setting_add(identify, NULL, CONFIG_TYPE_GROUP))) {
    fprintf(stderr, "swupdate:: Failed to add identify current_version\n");
    return false;
  }
  if (!(element = config_setting_add(setting, "name", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, "memfault__current_version")) {
    fprintf(stderr, "swupdate:: Failed to add identify current_version\n");
    return false;
  }
  if (!(element = config_setting_add(setting, "value", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, handle->software_version)) {
    fprintf(stderr, "swupdate:: Failed to add identify current_version\n");
    return false;
  }

  if (!(setting = config_setting_add(identify, NULL, CONFIG_TYPE_GROUP))) {
    fprintf(stderr, "swupdate:: Failed to add identify hardware_version\n");
    return false;
  }
  if (!(element = config_setting_add(setting, "name", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, "memfault__hardware_version")) {
    fprintf(stderr, "swupdate:: Failed to add identify hardware_version\n");
    return false;
  }
  if (!(element = config_setting_add(setting, "value", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, handle->hardware_version)) {
    fprintf(stderr, "swupdate:: Failed to add identify hardware_version\n");
    return false;
  }

  if (!(setting = config_setting_add(identify, NULL, CONFIG_TYPE_GROUP))) {
    fprintf(stderr, "swupdate:: Failed to add identify software_type\n");
    return false;
  }
  if (!(element = config_setting_add(setting, "name", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, "memfault__software_type")) {
    fprintf(stderr, "swupdate:: Failed to add identify software_type\n");
    return false;
  }
  if (!(element = config_setting_add(setting, "value", CONFIG_TYPE_STRING)) ||
      !config_setting_set_string(element, handle->software_type)) {
    fprintf(stderr, "swupdate:: Failed to add identify software_type\n");
    return false;
  }

  return true;
}

/**
 * @brief Generate new swupdate.cfg file from config
 *
 * @param handle swupdate config handle
 * @return true Successfully generated new config
 * @return false Failed to generate
 */
bool memfault_swupdate_generate_config(sMemfaultdSwupdateConfig *handle) {
  config_t config;

  config_init(&config);
  if (!config_read_file(&config, handle->input_file)) {
    fprintf(stderr,
            "swupdate:: Failed to read '%s', proceeding "
            "with defaults\n",
            handle->input_file);
  }

  if (!prv_swupdate_add_globals(&config)) {
    fprintf(stderr, "swupdate:: Failed to add global options to config\n");
    config_destroy(&config);
    return false;
  }
  if (!prv_swupdate_add_suricatta(handle, &config)) {
    fprintf(stderr, "swupdate:: Failed to add suricatta options to config\n");
    config_destroy(&config);
    return false;
  }
  if (!prv_swupdate_add_identify(handle, &config)) {
    fprintf(stderr, "swupdate:: Failed to add identify options to config\n");
    config_destroy(&config);
    return false;
  }

  if (!config_write_file(&config, handle->output_file)) {
    fprintf(stderr, "swupdate:: Failed to write config file to '%s'\n", handle->output_file);
    config_destroy(&config);
    return false;
  }

  config_destroy(&config);

  return true;
}
