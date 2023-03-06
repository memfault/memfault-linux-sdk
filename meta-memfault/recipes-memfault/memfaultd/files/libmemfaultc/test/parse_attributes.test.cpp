//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Unit tests for parse_attributes.c
//!

#include "../src/memfaultctl/parse_attributes.h"

#include <CppUTest/TestHarness.h>
#include <CppUTestExt/MockSupport.h>
#include <json-c/json.h>
#include <stdlib.h>
#include <string.h>

#include "memfault/core/math.h"

TEST_GROUP(TestParseAttributesGroup){};

extern "C" {}

TEST(TestParseAttributesGroup, BasicStrings) {
  json_object *json;

  const char *argv[] = {"VAR1=VALUE1", "VAR2=VALUE2", "VAR3=VALUE3"};

  CHECK(memfaultd_parse_attributes(argv, MEMFAULT_ARRAY_SIZE(argv), &json));

  STRCMP_EQUAL("[ "
               "{ \"string_key\": \"VAR1\", \"value\": \"VALUE1\" }, "
               "{ \"string_key\": \"VAR2\", \"value\": \"VALUE2\" }, "
               "{ \"string_key\": \"VAR3\", \"value\": \"VALUE3\" } "
               "]",
               json_object_to_json_string(json));

  json_object_put(json);
}

TEST(TestParseAttributesGroup, EmptyAttributes) {
  json_object *json;

  const char *argv[] = {};

  CHECK(memfaultd_parse_attributes(argv, MEMFAULT_ARRAY_SIZE(argv), &json) == false);

  CHECK(json == NULL);
}

TEST(TestParseAttributesGroup, InvalidAttributes) {
  json_object *json;

  const char *argv[] = {"VARIABLE", "=", "SOMETHING"};

  CHECK(memfaultd_parse_attributes(argv, MEMFAULT_ARRAY_SIZE(argv), &json) == false);
  CHECK(json == NULL);
}

TEST(TestParseAttributesGroup, ComboValidInvalid) {
  json_object *json;

  const char *argv[] = {"V1=X", "V2"};

  CHECK(memfaultd_parse_attributes(argv, MEMFAULT_ARRAY_SIZE(argv), &json) == false);
  CHECK(json == NULL);
}

#define TEST_VALUE(name, value, json_value)                                    \
  TEST(TestParseAttributesGroup, name) {                                       \
    json_object *json;                                                         \
    const char *argv[] = {"v1=" value};                                        \
    CHECK(memfaultd_parse_attributes(argv, MEMFAULT_ARRAY_SIZE(argv), &json)); \
    STRCMP_EQUAL("[ "                                                          \
                 "{ \"string_key\": \"v1\", \"value\": " json_value " } "      \
                 "]",                                                          \
                 json_object_to_json_string(json));                            \
                                                                               \
    json_object_put(json);                                                     \
  }

TEST_VALUE(string, "abc", "\"abc\"")
TEST_VALUE(valueWithEqual, "abc=def", "\"abc=def\"")
TEST_VALUE(quotedString, "\"quoted\"", "\"quoted\"")
TEST_VALUE(boolvalue, "false", "false")
TEST_VALUE(boolvalueAsString, "\"false\"", "\"false\"")
TEST_VALUE(integer, "42", "42")
TEST_VALUE(integerAsString, "\"42\"", "\"42\"")
TEST_VALUE(floating, "42.1", "42.1")
TEST_VALUE(floatAsString, "\"42.1\"", "\"42.1\"")

// We use a JSON parser but we do not want to allow array or object types
// Make sure an escaped version of the input string is returned instead.
TEST_VALUE(json_array, "[1,2,3]", "\"[1,2,3]\"")
TEST_VALUE(json_object, "{ \"a\": 1 }", "\"{ \\\"a\\\": 1 }\"")
