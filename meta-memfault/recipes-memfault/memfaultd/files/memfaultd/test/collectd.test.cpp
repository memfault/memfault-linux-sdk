//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for queue.c
//!

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>
#include <fcntl.h>
#include <json-c/json.h>
#include <libuboot.h>
#include <systemd/sd-bus.h>
#include <unistd.h>

#include <fstream>
#include <iostream>
#include <sstream>

#include "memfaultd.h"
#include "memfaultd_utils.h"

static sMemfaultd *g_stub_memfaultd = (sMemfaultd *)~0;

extern "C" {
bool memfaultd_collectd_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
}

const sMemfaultdDeviceSettings *memfaultd_get_device_settings(sMemfaultd *memfaultd) {
  return (sMemfaultdDeviceSettings *)mock()
    .actualCall("memfaultd_get_device_settings")
    .returnConstPointerValue();
}

bool memfaultd_utils_restart_service_if_running(const char *src_module, const char *service_name) {
  return mock()
    .actualCall("memfaultd_utils_restart_service_if_running")
    .withStringParameter("src_module", src_module)
    .withStringParameter("service_name", service_name)
    .returnBoolValue();
}

bool memfaultd_get_boolean(sMemfaultd *handle, const char *parent_key, const char *key, bool *val) {
  return mock()
    .actualCall("memfaultd_get_boolean")
    .withStringParameter("parent_key", parent_key)
    .withStringParameter("key", key)
    .withOutputParameter("val", val)
    .returnBoolValue();
}

bool memfaultd_get_integer(sMemfaultd *handle, const char *parent_key, const char *key, int *val) {
  return mock()
    .actualCall("memfaultd_get_integer")
    .withStringParameter("parent_key", parent_key)
    .withStringParameter("key", key)
    .withOutputParameter("val", val)
    .returnBoolValue();
}

bool memfaultd_get_string(sMemfaultd *handle, const char *parent_key, const char *key,
                          const char **val) {
  return mock()
    .actualCall("memfaultd_get_string")
    .withStringParameter("parent_key", parent_key)
    .withStringParameter("key", key)
    .withOutputParameter("val", val)
    .returnBoolValue();
}

TEST_BASE(MemfaultdCollectdUtest) {
  sMemfaultdPluginCallbackFns *fns = NULL;
  bool comms_enabled = false;
  char *header_include_output_file = NULL;
  char *footer_include_output_file = NULL;

  char device_id[32] = "device_id";
  char hardware_version[32] = "hw_version";
  const sMemfaultdDeviceSettings deviceSettings = {.device_id = device_id,
                                                   .hardware_version = hardware_version};
  char *base_url = NULL;
  char *software_type = NULL;
  char *software_version = NULL;
  char *project_key = NULL;

  int write_http_buf_size = 64;
  int interval_seconds = 3600;
  char *non_memfaultd_chain = NULL;

  char tmp_dir[PATH_MAX] = {0};
  char tmp_header_include_output_file[4200] = {0};
  char tmp_footer_include_output_file[4200] = {0};

  void setup() override {
    strcpy(tmp_dir, "/tmp/memfaultd.XXXXXX");
    mkdtemp(tmp_dir);
    sprintf(tmp_header_include_output_file, "%s/collectd-header-include.conf", tmp_dir);
    sprintf(tmp_footer_include_output_file, "%s/collectd-footer-include.conf", tmp_dir);
  }

  void teardown() override {
    unlink(tmp_header_include_output_file);
    unlink(tmp_footer_include_output_file);
    rmdir(tmp_dir);
    free(header_include_output_file);
    free(footer_include_output_file);
    free(base_url);
    free(software_type);
    free(software_version);
    free(project_key);
    free(non_memfaultd_chain);
    mock().checkExpectations();
    mock().clear();
  }

  int get_file_size(const char *file) {
    std::ifstream in(file, std::ifstream::ate | std::fstream::binary);
    return in.tellg();
  }

  char *get_file_content(const char *file) {
    std::ifstream in(file);
    std::stringstream buf;
    buf << in.rdbuf();
    return strdup(buf.str().c_str());
  }

  void expect_enable_data_collection_get_boolean_call(bool val) {
    comms_enabled = val;
    mock()
      .expectOneCall("memfaultd_get_boolean")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "enable_data_collection")
      .withOutputParameterReturning("val", &comms_enabled, sizeof(comms_enabled))
      .andReturnValue(true);
  }

  void expect_collectd_header_include_output_file_get_string_call(const char *val) {
    header_include_output_file = strdup(val);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "collectd_plugin")
      .withStringParameter("key", "header_include_output_file")
      .withOutputParameterReturning("val", &header_include_output_file,
                                    sizeof(header_include_output_file))
      .andReturnValue(true);
  }

  void expect_collectd_footer_include_output_file_get_string_call(const char *val) {
    footer_include_output_file = strdup(val);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "collectd_plugin")
      .withStringParameter("key", "footer_include_output_file")
      .withOutputParameterReturning("val", &footer_include_output_file,
                                    sizeof(footer_include_output_file))
      .andReturnValue(true);
  }

  void expect_memfaultd_utils_restart_service_if_running_call() {
    mock()
      .expectOneCall("memfaultd_utils_restart_service_if_running")
      .withStringParameter("src_module", "collectd")
      .withStringParameter("service_name", "collectd.service")
      .andReturnValue(true);
  }

  void expect_get_all_device_settings() {
    mock().expectOneCall("memfaultd_get_device_settings").andReturnValue(&deviceSettings);
    base_url = strdup("https://example.com");
    software_type = strdup("sw_type");
    software_version = strdup("123.456");
    project_key = strdup("projectkey");
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "base_url")
      .withOutputParameterReturning("val", &base_url, sizeof(base_url))
      .andReturnValue(true);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "software_type")
      .withOutputParameterReturning("val", &software_type, sizeof(software_type))
      .andReturnValue(true);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "software_version")
      .withOutputParameterReturning("val", &software_version, sizeof(software_version))
      .andReturnValue(true);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "project_key")
      .withOutputParameterReturning("val", &project_key, sizeof(project_key))
      .andReturnValue(true);
  }

  void expect_get_all_collectd_settings() {
    non_memfaultd_chain = strdup("");
    mock()
      .expectNCalls(2, "memfaultd_get_integer")
      .withStringParameter("parent_key", "collectd_plugin")
      .withStringParameter("key", "interval_seconds")
      .withOutputParameterReturning("val", &interval_seconds, sizeof(interval_seconds))
      .andReturnValue(true);
    mock()
      .expectOneCall("memfaultd_get_integer")
      .withStringParameter("parent_key", "collectd_plugin")
      .withStringParameter("key", "write_http_buffer_size_kib")
      .withOutputParameterReturning("val", &write_http_buf_size, sizeof(write_http_buf_size))
      .andReturnValue(true);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "collectd_plugin")
      .withStringParameter("key", "non_memfaultd_chain")
      .withOutputParameterReturning("val", &non_memfaultd_chain, sizeof(non_memfaultd_chain))
      .andReturnValue(true);
  }
};

TEST_GROUP_BASE(TestGroup_Startup, MemfaultdCollectdUtest){};

/* comms disabled; init succeeds, empty config file */
TEST(TestGroup_Startup, Test_DataCommsDisabled) {
  expect_enable_data_collection_get_boolean_call(false);  // startup collection enabled state
  expect_enable_data_collection_get_boolean_call(false);  // current collection enabled state
  expect_collectd_header_include_output_file_get_string_call(
    tmp_header_include_output_file);  // header include filename
  expect_collectd_footer_include_output_file_get_string_call(
    tmp_footer_include_output_file);  // footer include filename

  bool success = memfaultd_collectd_init(g_stub_memfaultd, &fns);

  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);
  CHECK_EQUAL(0, get_file_size(tmp_header_include_output_file));
  CHECK_EQUAL(0, get_file_size(tmp_footer_include_output_file));

  free(fns->handle);
}

/* comms enabled; init succeeds, populated config file */
TEST(TestGroup_Startup, Test_DataCommsEnabled) {
  const char *expected_header_include_file = "Interval 3600\n"
                                             "\n";

  const char *expected_footer_include_file =
    "<LoadPlugin write_http>\n"
    "  FlushInterval 3600\n"
    "</LoadPlugin>\n"
    "\n"
    "<Plugin write_http>\n"
    "  <Node \"memfault\">\n"
    "    URL \"https://example.com/api/v0/collectd/device_id/hw_version/sw_type/123.456\"\n"
    "    VerifyPeer true\n"
    "    VerifyHost true\n"
    "    Header \"Memfault-Project-Key: projectkey\"\n"
    "    Format \"JSON\"\n"
    "    Metrics true\n"
    "    Notifications false\n"
    "    StoreRates true\n"
    "    BufferSize 65536\n"
    "    LowSpeedLimit 0\n"
    "    Timeout 0\n"
    "  </Node>\n"
    "</Plugin>\n"
    "\n"
    "LoadPlugin match_regex\n"
    "PostCacheChain \"MemfaultdGeneratedPostCacheChain\"\n"
    "<Chain \"MemfaultdGeneratedPostCacheChain\">\n"
    "  <Rule \"ignore_memory_metrics\">\n"
    "    <Match \"regex\">\n"
    "      Type \"^memory$\"\n"
    "      TypeInstance \"^(buffered|cached|slab_recl|slab_unrecl)$\"\n"
    "    </Match>\n"
    "    Target \"stop\"\n"
    "  </Rule>\n"
    "  Target \"write\"\n"
    "</Chain>\n\n";

  expect_enable_data_collection_get_boolean_call(true);  // startup collection enabled state
  expect_enable_data_collection_get_boolean_call(true);  // current collection enabled state
  expect_collectd_header_include_output_file_get_string_call(
    tmp_header_include_output_file);  // header include filename
  expect_collectd_footer_include_output_file_get_string_call(
    tmp_footer_include_output_file);  // footer include filename

  expect_get_all_device_settings();  // For URL creation
  expect_get_all_collectd_settings();
  expect_memfaultd_utils_restart_service_if_running_call();

  bool success = memfaultd_collectd_init(g_stub_memfaultd, &fns);

  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  CHECK(get_file_size(tmp_header_include_output_file) != 0);
  char *header_payload = get_file_content(tmp_header_include_output_file);
  STRCMP_EQUAL(expected_header_include_file, header_payload);
  free(header_payload);

  CHECK(get_file_size(tmp_footer_include_output_file) != 0);
  char *footer_payload = get_file_content(tmp_footer_include_output_file);
  STRCMP_EQUAL(expected_footer_include_file, footer_payload);
  free(footer_payload);

  free(fns->handle);
}
