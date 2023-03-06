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
#include <assert.h>
#include <dlfcn.h>
#include <json-c/json.h>
#include <unistd.h>

#include <fstream>
#include <iostream>
#include <sstream>

#include "memfault/util/linux_boot_id.h"
#include "memfaultd.h"
#include "reboot/reboot_last_boot_id.h"
#include "reboot/reboot_process_pstore.h"

extern "C" {
#include <libuboot.h>
#include <systemd/sd-bus.h>
}

static sMemfaultd *g_stub_memfaultd = (sMemfaultd *)~0;
static sMemfaultdTxData *g_stub_txdata = NULL;

extern "C" {
bool memfaultd_reboot_init(sMemfaultd *memfaultd, sMemfaultdPluginCallbackFns **fns);
void memfaultd_reboot_data_collection_enabled(sMemfaultd *memfaultd, bool data_collection_enabled);

// Override libc's access() with a mock
int access(const char *pathname, int mode) { return mock().actualCall("access").returnIntValue(); }
// Provide a way to call the real access libc function
int __real_access(const char *pathname, int mode) {
  int (*real_access)(const char *, int) = (int (*)(const char *, int))dlsym(RTLD_NEXT, "access");

  assert(real_access);

  return real_access(pathname, mode);
}
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

bool memfault_reboot_is_untracked_boot_id(const char *last_tracked_boot_id_file,
                                          const char *current_boot_id) {
  return mock().actualCall("memfault_reboot_is_untracked_boot_id").returnBoolValue();
}

void memfault_reboot_process_pstore_files(char *pstore_dir) {
  mock().actualCall("memfault_reboot_process_pstore_files");
}

bool memfault_linux_boot_id_read(char boot_id[UUID_STR_LEN]) {
  strcpy(boot_id, "12764a0c-f27b-48b3-8fe2-10fa14fa1917");
  return true;
}

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

sd_bus *sd_bus_unref(sd_bus *bus) { return bus; }

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
  char tmp_customer_reboot_file[4200] = {0};
  const char *tmp_customer_reboot_file_ptr;

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
    snprintf(tmp_reboot_file, sizeof(tmp_reboot_file), "%s/lastrebootreason", tmp_dir);
    snprintf(tmp_customer_reboot_file, sizeof(tmp_customer_reboot_file),
             "%s/customer_last_reboot_reason", tmp_dir);
    tmp_customer_reboot_file_ptr = tmp_customer_reboot_file;
  }

  void teardown() override {
    if (g_stub_txdata) {
      free(g_stub_txdata);
      g_stub_txdata = NULL;
    }
    tmp_customer_reboot_file_ptr = nullptr;
    unlink(tmp_customer_reboot_file);
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

  void expect_customer_reboot_reason_file_get_string_call() {
    mock()
      .expectOneCall("memfaultd_get_string")
      .withStringParameter("parent_key", "reboot_plugin")
      .withStringParameter("key", "last_reboot_reason_file")
      .withOutputParameterReturning("val", &tmp_customer_reboot_file_ptr,
                                    sizeof(tmp_customer_reboot_file_ptr))
      .andReturnValue(true);
  }

  void expect_lastrebootreason_file_generate_call(const char *path) {
    mock()
      .expectOneCall("memfaultd_generate_rw_filename")
      .withPointerParameter("memfaultd", g_stub_memfaultd)
      .withStringParameter("filename", "lastrebootreason")
      .andReturnValue(tmp_reboot_file);
  }

  void expect_last_tracked_boot_id_file_generate_call() {
    mock()
      .expectOneCall("memfaultd_generate_rw_filename")
      .withPointerParameter("memfaultd", g_stub_memfaultd)
      .withStringParameter("filename", "last_tracked_boot_id")
      .andReturnValue("/last_tracked_boot_id");
  }

  void expect_memfault_reboot_is_untracked_boot_id(bool is_untracked) {
    mock().expectOneCall("memfault_reboot_is_untracked_boot_id").andReturnValue(is_untracked);
  }

  void expect_get_device_settings_call() {
    mock().expectOneCall("memfaultd_get_device_settings").andReturnValue(&deviceSettings);
  }

  void expect_memfaultd_txdata_call(void) {
    mock().expectOneCall("memfaultd_txdata").andReturnValue(true);
  }

  void expect_access_call(int val) { mock().expectOneCall("access").andReturnValue(val); }

  void expect_memfault_reboot_process_pstore_files_call(void) {
    mock().expectOneCall("memfault_reboot_process_pstore_files");
  }

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

  void expect_tx_unknown_reboot_reason() {
    expect_get_device_settings_call();                         // get device info (hw version etc)
    expect_software_type_get_string_call("my_software_type");  // get software_type
    expect_software_version_get_string_call("my_software_version");  // get software_version
    expect_memfaultd_txdata_call();                                  // tx data call
  }

  static void check_last_txdata_has_reboot_reason(int reboot_reason) {
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
    CHECK_EQUAL(json_object_get_int(object), reboot_reason);

    json_object_put(payload);
  }

  static void check_file_does_not_exist(const char *path) {
    CHECK_EQUAL(-1, __real_access(path, F_OK));
    CHECK_EQUAL(ENOENT, errno);
  }

  void write_lastrebootreason_file(const char *val) {
    std::ofstream fd(tmp_reboot_file);
    fd << val;
  }

  void write_customer_last_reboot_reason_file(const char *val) {
    std::ofstream fd(tmp_customer_reboot_file);
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
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_memfault_reboot_process_pstore_files_call();

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(!fns);
  CHECK_TRUE(g_stub_txdata == nullptr);
}

/* comms enabled, no reboot reason file; init returns true with destroy function and
 * valid handle, txdata */
TEST(TestGroup_Startup, Test_DataCommsEnabled) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call(
    "lastrebootreason");   // read missing reboot reason file
  expect_access_call(-1);  // no pstore file
  expect_tx_unknown_reboot_reason();

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  check_last_txdata_has_reboot_reason(0);  // kMfltRebootReason_Unknown

  free(fns->handle);
}

/* empty reboot reason file; txdata, init successful */
TEST(TestGroup_Startup, Test_EmptyRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  write_lastrebootreason_file("");                                 // write empty reboot reason file
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_tx_unknown_reboot_reason();

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  check_last_txdata_has_reboot_reason(0);      // kMfltRebootReason_Unknown
  check_file_does_not_exist(tmp_reboot_file);  // File is deleted after having been read

  free(fns->handle);
}

/* invalid reboot reason file; txdata, init successful */
TEST(TestGroup_Startup, Test_InvalidRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  write_lastrebootreason_file("notANumber");  // write invalid reboot reason file
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file

  expect_tx_unknown_reboot_reason();

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);
  CHECK(fns);
  CHECK(fns->handle);
  CHECK(fns->plugin_destroy);

  check_last_txdata_has_reboot_reason(0);      // kMfltRebootReason_Unknown
  check_file_does_not_exist(tmp_reboot_file);  // File is deleted after having been read

  free(fns->handle);
}

/* valid internal reboot reason file; txdata, init successful */
TEST(TestGroup_Startup, Test_ValidRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  write_lastrebootreason_file("2");  // write valid reboot reason file (2=user reset)
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file

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

  check_last_txdata_has_reboot_reason(2);      // kMfltRebootReason_UserReset
  check_file_does_not_exist(tmp_reboot_file);  // File is deleted after having been read

  free(fns->handle);
}

/* valid customer reboot reason file; txdata, init successful */
TEST(TestGroup_Startup, Test_ValidCustomerRebootReasonFile) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file

  write_customer_last_reboot_reason_file(
    "0");  // write valid customer reboot reason file (0=unknown)

  // Customer reboot reason file trumps the internal reboot reason file:
  write_lastrebootreason_file("2");  // write valid reboot reason file (2=user reset)

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

  check_last_txdata_has_reboot_reason(0);               // kMfltRebootReason_Unknown
  check_file_does_not_exist(tmp_reboot_file);           // File is deleted after having been read
  check_file_does_not_exist(tmp_customer_reboot_file);  // File is deleted after having been read

  free(fns->handle);
}

/* pstore panic file found; txdata, init successful */
TEST(TestGroup_Startup, Test_PstorePanic) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(0);                                           // pstore file found
  expect_memfault_reboot_process_pstore_files_call();

  // Pstore/kernel panic reboot reason trumps the internal and customer reboot reason files:
  write_lastrebootreason_file("3");  // write valid reboot reason file (3=firmware update)
  write_customer_last_reboot_reason_file(
    "4");  // write valid customer reboot reason file (4=low power)

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

  check_last_txdata_has_reboot_reason(0x8008);  // kMfltRebootReason_KernelPanic

  check_file_does_not_exist(tmp_reboot_file);           // File is deleted after having been read
  check_file_does_not_exist(tmp_customer_reboot_file);  // File is deleted after having been read

  free(fns->handle);
}

/* no pstore panic file, no internal file, no customer file; starting; txdata, init successful */
TEST(TestGroup_Startup, Test_StartupUnknownReason) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file

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

  check_last_txdata_has_reboot_reason(0);  // kMfltRebootReason_Unknown

  free(fns->handle);
}

TEST_GROUP_BASE(TestGroup_Shutdown, MemfaultdRebootUtest){};

/* not shutting down; no file */
TEST(TestGroup_Shutdown, Test_StillRunning) {
  // 'Empty' startup
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_tx_unknown_reboot_reason();

  memfaultd_reboot_init(g_stub_memfaultd, &fns);

  expect_sd_bus_get_property_string_call("running");  // systemd is still in running state

  fns->plugin_destroy(fns->handle);
}

/* shutting down, not upgrade; "2" in lastrebootreason */
TEST(TestGroup_Shutdown, Test_Stopping) {
  // 'Empty' startup
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_tx_unknown_reboot_reason();

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
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(true);
  expect_customer_reboot_reason_file_get_string_call();
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // read reboot reason file
  expect_access_call(-1);                                          // no pstore file
  expect_tx_unknown_reboot_reason();

  memfaultd_reboot_init(g_stub_memfaultd, &fns);

  expect_sd_bus_get_property_string_call("stopping");              // systemd in stopping state
  expect_ustate_calls("1");                                        // upgrading
  expect_lastrebootreason_file_generate_call("lastrebootreason");  // write reboot reason file

  fns->plugin_destroy(fns->handle);

  char *lastrebootreason_str = read_lastrebootreason_file();
  MEMCMP_EQUAL("3", lastrebootreason_str, strlen(lastrebootreason_str));
  free(lastrebootreason_str);
}

/* boot_id already tracked, the plugin does nothing on init in this case */
TEST(TestGroup_Startup, Test_BootIdAlreadyTracked) {
  expect_enable_data_collection_get_boolean_call(true);  // collection enabled
  expect_last_tracked_boot_id_file_generate_call();
  expect_memfault_reboot_is_untracked_boot_id(false);

  bool success = memfaultd_reboot_init(g_stub_memfaultd, &fns);
  CHECK_EQUAL(success, true);

  CHECK_TRUE(g_stub_txdata == nullptr);

  free(fns->handle);
}
