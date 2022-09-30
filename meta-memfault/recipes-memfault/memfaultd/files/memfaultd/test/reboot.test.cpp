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

static sMemfaultd *g_stub_memfaultd = (sMemfaultd *)~0;
static sMemfaultdTxData *g_stub_txdata = NULL;

extern "C" {
bool memfaultd_reboot_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
}

char *memfaultd_generate_rw_filename(sMemfaultd *memfaultd, const char *filename) {
  const char *path = mock()
                       .actualCall("memfaultd_generate_rw_filename")
                       .withPointerParameter("memfaultd", memfaultd)
                       .withStringParameter("filename", filename)
                       .returnStringValue();
  return strdup(path);  // original returns malloc'd string
}

bool memfaultd_get_boolean(sMemfaultd *handle, const char *parent_key, const char *key, bool *val) {
  return mock()
    .actualCall("memfaultd_get_boolean")
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

int access(const char *pathname, int mode) { return mock().actualCall("access").returnIntValue(); }

const sMemfaultdDeviceSettings *memfaultd_get_device_settings(sMemfaultd *memfaultd) {
  return (sMemfaultdDeviceSettings *)mock()
    .actualCall("memfaultd_get_device_settings")
    .returnConstPointerValue();
}

bool memfaultd_txdata(sMemfaultd *memfaultd, const sMemfaultdTxData *data, uint32_t payload_size) {
  g_stub_txdata = (sMemfaultdTxData *)malloc(sizeof(sMemfaultdTxData) + payload_size);
  memcpy(g_stub_txdata, data, sizeof(sMemfaultdTxData) + payload_size);

  return mock().actualCall("memfaultd_txdata").returnBoolValue();
}

int sd_bus_default_system(sd_bus **bus) {
  return mock().actualCall("sd_bus_default_system").returnIntValue();
}

int sd_bus_get_property_string(sd_bus *bus, const char *destination, const char *path,
                               const char *interface, const char *member, sd_bus_error *ret_error,
                               char **ret) {
  return mock()
    .actualCall("sd_bus_get_property_string")
    .withOutputParameter("ret", ret)
    .returnIntValue();
}

void sd_bus_error_free(sd_bus_error *e) {}

int libuboot_initialize(struct uboot_ctx **out, struct uboot_env_device *envdevs) {
  return mock().actualCall("libuboot_initialize").returnIntValue();
}

int libuboot_read_config(struct uboot_ctx *ctx, const char *config) {
  return mock().actualCall("libuboot_read_config").returnIntValue();
}

int libuboot_open(struct uboot_ctx *ctx) {
  return mock().actualCall("libuboot_open").returnIntValue();
}

char *libuboot_get_env(struct uboot_ctx *ctx, const char *varname) {
  return (char *)mock().actualCall("libuboot_get_env").returnPointerValue();
}

void libuboot_close(struct uboot_ctx *ctx) { mock().actualCall("libuboot_close"); }

void libuboot_exit(struct uboot_ctx *ctx) { mock().actualCall("libuboot_exit"); }

TEST_BASE(MemfaultdRebootUtest) {
  char tmp_dir[PATH_MAX] = {0};
  char tmp_reboot_file[4200] = {0};

  sMemfaultdPluginCallbackFns *fns = NULL;
  char device_id[32] = "my_device_id";
  char hardware_version[32] = "my_hardware_version";
  const sMemfaultdDeviceSettings deviceSettings = {.device_id = device_id,
                                                   .hardware_version = hardware_version};

  bool comms_enabled;
  const char *software_type;
  const char *software_version;
  const sMemfaultdTxData *data;
  const char *system_state;
  const char *ustate;

  void setup() override {
    strcpy(tmp_dir, "/tmp/memfaultd.XXXXXX");
    mkdtemp(tmp_dir);
    sprintf(tmp_reboot_file, "%s/lastrebootreason", tmp_dir);
  }

  void teardown() override {
    if (g_stub_txdata) {
      free(g_stub_txdata);
      g_stub_txdata = NULL;
    }
    unlink(tmp_reboot_file);
    rmdir(tmp_dir);
    mock().checkExpectations();
    mock().clear();
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

  void expect_software_type_get_string_call(const char *val) {
    software_type = val;
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "software_type")
      .withOutputParameterReturning("val", &software_type, sizeof(software_type))
      .andReturnValue(true);
  }

  void expect_software_version_get_string_call(const char *val) {
    software_version = val;
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "")
      .withStringParameter("key", "software_version")
      .withOutputParameterReturning("val", &software_version, sizeof(software_version))
      .andReturnValue(true);
  }

  void expect_lastrebootreason_file_generate_call(const char *path) {
    mock()
      .expectOneCall("memfaultd_generate_rw_filename")
      .withPointerParameter("memfaultd", g_stub_memfaultd)
      .withStringParameter("filename", "lastrebootreason")
      .andReturnValue(tmp_reboot_file);
  }

  void expect_get_device_settings_call() {
    mock().expectOneCall("memfaultd_get_device_settings").andReturnValue(&deviceSettings);
  }

  void expect_memfaultd_txdata_call(void) {
    mock().expectOneCall("memfaultd_txdata").andReturnValue(true);
  }

  void expect_access_call(int val) { mock().expectOneCall("access").andReturnValue(val); }

  void expect_sd_bus_get_property_string_call(const char *val) {
    system_state = strdup(val);
    mock().expectOneCall("sd_bus_default_system").andReturnValue(0);
    mock()
      .expectOneCall("sd_bus_get_property_string")
      .withOutputParameterReturning("ret", &system_state, sizeof(system_state))
      .andReturnValue(0);
  }

  void expect_ustate_calls(const char *val) {
    ustate = strdup(val);
    mock().expectOneCall("libuboot_initialize").andReturnValue(0);
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "reboot_plugin")
      .withStringParameter("key", "uboot_fw_env_file")
      .withOutputParameterReturning("val", NULL, 0)
      .andReturnValue(true);
    mock().expectOneCall("libuboot_read_config").andReturnValue(0);
    mock().expectOneCall("libuboot_open").andReturnValue(0);

    mock().expectOneCall("libuboot_get_env").andReturnValue((void *)ustate);
    mock().expectOneCall("libuboot_close").andReturnValue(0);
    mock().expectOneCall("libuboot_exit").andReturnValue(0);
  }

  void write_lastrebootreason_file(const char *val) {
    std::ofstream fd(tmp_reboot_file);
    fd << val;
  }

  char *read_lastrebootreason_file() {
    std::ifstream fd(tmp_reboot_file);
    std::stringstream buf;
    buf << fd.rdbuf();
    return strdup(buf.str().c_str());
  }
};

TEST_GROUP_BASE(TestGroup_Startup, MemfaultdRebootUtest){};

/* comms disabled; init returns true with empty function table */
TEST(TestGroup_Startup, Test_DataCommsDisabled) {
  expect_enable_data_collection_get_boolean_call(false);  // collection disabled

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(!fns);
}

/* comms enabled, no reboot reason file; init returns true with destroy function and
 * valid handle, no txdata */
TEST(TestGroup_Startup, Test_DataCommsEnabled) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_lastrebootreason_file_generate_call(
    "lastrebootreason");                              // read missing reboot reason file
  expect_access_call(-1);                             // no pstore file
  expect_sd_bus_get_property_string_call("running");  // systemd in running state

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);
  free(fns->handle);
}

/* empty reboot reason file; no txdata, init successful */
TEST(TestGroup_Startup, Test_EmptyRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  write_lastrebootreason_file("");                                 // write empty reboot reason file
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("running");               // systemd in running state

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);
  free(fns->handle);
}

/* invalid reboot reason file; no txdata, init successful */
TEST(TestGroup_Startup, Test_InvalidRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  write_lastrebootreason_file("notANumber");             // write invalid reboot reason file
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("running");               // systemd in running state

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);
  free(fns->handle);
}

/* valid reboot reason file; txdata, init successful */
TEST(TestGroup_Startup, Test_ValidRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  write_lastrebootreason_file("2");  // write valid reboot reason file (2=user reset)
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file

  // Additional calls expected as tx'ing data
  expect_get_device_settings_call();                         // get device info (hw version etc)
  expect_software_type_get_string_call("my_software_type");  // get software_type
  expect_software_version_get_string_call("my_software_version");  // get software_version
  expect_memfaultd_txdata_call();                                  // tx data call

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  // Check object is valid
  json_object *payload, *object;
  CHECK_EQUAL(g_stub_txdata->type, kMemfaultdTxDataType_RebootEvent);
  payload = json_tokener_parse((const char *)g_stub_txdata->payload);
  CHECK(payload);

  // Object is an array or length 1, get first entry
  CHECK_EQUAL(json_object_get_type(payload), json_type_array);
  CHECK_EQUAL(json_object_array_length(payload), 1);
  object = json_object_array_get_idx(payload, 0);
  CHECK(object);

  // Get reboot reason
  CHECK(json_object_object_get_ex(object, "event_info", &object));
  CHECK_EQUAL(json_object_get_type(object), json_type_object);
  CHECK(json_object_object_get_ex(object, "reason", &object));
  CHECK_EQUAL(json_object_get_type(object), json_type_int);
  CHECK_EQUAL(json_object_get_int(object), 2);

  json_object_put(payload);
  free(fns->handle);
}

/* pstore panic file found; txdata, init successful */
TEST(TestGroup_Startup, Test_PstorePanic) {
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(0);                                           // pstore file found

  // Additional calls expected as tx'ing data
  expect_get_device_settings_call();                         // get device info (hw version etc)
  expect_software_type_get_string_call("my_software_type");  // get software_type
  expect_software_version_get_string_call("my_software_version");  // get software_version
  expect_memfaultd_txdata_call();                                  // tx data call

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  // Check object is valid
  json_object *payload, *object;
  CHECK_EQUAL(g_stub_txdata->type, kMemfaultdTxDataType_RebootEvent);
  payload = json_tokener_parse((const char *)g_stub_txdata->payload);
  CHECK(payload);

  // Object is an array or length 1, get first entry
  CHECK_EQUAL(json_object_get_type(payload), json_type_array);
  CHECK_EQUAL(json_object_array_length(payload), 1);
  object = json_object_array_get_idx(payload, 0);
  CHECK(object);

  // Get reboot reason
  CHECK(json_object_object_get_ex(object, "event_info", &object));
  CHECK_EQUAL(json_object_get_type(object), json_type_object);
  CHECK(json_object_object_get_ex(object, "reason", &object));
  CHECK_EQUAL(json_object_get_type(object), json_type_int);
  CHECK_EQUAL(json_object_get_int(object), 32771);

  json_object_put(payload);
  free(fns->handle);
}

/* no pstore panic file, already running; no txdata, successful */
TEST(TestGroup_Startup, Test_AlreadyRunning) {
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("running");               // systemd in running state

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);
  free(fns->handle);
}

/* no pstore panic file, starting; txdata, init successful */
TEST(TestGroup_Startup, Test_Startup) {
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("starting");              // systemd in starting state

  // Additional calls expected as tx'ing data
  expect_get_device_settings_call();                         // get device info (hw version etc)
  expect_software_type_get_string_call("my_software_type");  // get software_type
  expect_software_version_get_string_call("my_software_version");  // get software_version
  expect_memfaultd_txdata_call();                                  // tx data call

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  // Check object is valid
  json_object *payload, *object;
  CHECK_EQUAL(g_stub_txdata->type, kMemfaultdTxDataType_RebootEvent);
  payload = json_tokener_parse((const char *)g_stub_txdata->payload);
  CHECK(payload);

  // Object is an array or length 1, get first entry
  CHECK_EQUAL(json_object_get_type(payload), json_type_array);
  CHECK_EQUAL(json_object_array_length(payload), 1);
  object = json_object_array_get_idx(payload, 0);
  CHECK(object);

  // Get reboot reason
  CHECK(json_object_object_get_ex(object, "event_info", &object));
  CHECK_EQUAL(json_object_get_type(object), json_type_object);
  CHECK(json_object_object_get_ex(object, "reason", &object));
  CHECK_EQUAL(json_object_get_type(object), json_type_int);
  CHECK_EQUAL(json_object_get_int(object), 4);

  json_object_put(payload);
  free(fns->handle);
}

TEST_GROUP_BASE(TestGroup_Shutdown, MemfaultdRebootUtest){};

/* not shutting down; no file */
TEST(TestGroup_Shutdown, Test_StillRunning) {
  // 'Empty' startup
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("running");               // systemd in running state

  memfaultd_reboot_init(g_stub_memfaultd, &fns);

  expect_sd_bus_get_property_string_call("running");  // systemd is still in running state

  fns->plugin_destroy(fns->handle);
}

/* shutting down, not upgrade; "2" in lastrebootreason */
TEST(TestGroup_Shutdown, Test_Stopping) {
  // 'Empty' startup
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("running");               // systemd in running state

  memfaultd_reboot_init(g_stub_memfaultd, &fns);

  expect_sd_bus_get_property_string_call("stopping");              // systemd in stopping state
  expect_ustate_calls("0");                                        // not upgrading
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // write reboot reason file

  fns->plugin_destroy(fns->handle);

  char *lastrebootreason_str = read_lastrebootreason_file();
  MEMCMP_EQUAL("2", lastrebootreason_str, strlen(lastrebootreason_str));
  free(lastrebootreason_str);
}

/* shutting down, upgrade; "3" in lastrebootreason */
TEST(TestGroup_Shutdown, Test_Upgrade) {
  // 'Empty' startup
  expect_enable_data_collection_get_boolean_call(true);            // collection enabled
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_sd_bus_get_property_string_call("running");               // systemd in running state

  memfaultd_reboot_init(g_stub_memfaultd, &fns);

  expect_sd_bus_get_property_string_call("stopping");              // systemd in stopping state
  expect_ustate_calls("1");                                        // upgrading
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // write reboot reason file

  fns->plugin_destroy(fns->handle);

  char *lastrebootreason_str = read_lastrebootreason_file();
  MEMCMP_EQUAL("3", lastrebootreason_str, strlen(lastrebootreason_str));
  free(lastrebootreason_str);
}
