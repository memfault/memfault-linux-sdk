#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! My utilities.

#include <json-c/json.h>

#ifdef __cplusplus
extern "C" {
#endif

//! @brief json-c after v0.16.00 deprecated JSON_C_OBJECT_KEY_IS_CONSTANT in favor of
//! JSON_C_OBJECT_ADD_CONSTANT_KEY. This provides a compatibility shim for older versions of json-c.
#ifndef JSON_C_OBJECT_ADD_CONSTANT_KEY
  #define JSON_C_OBJECT_ADD_CONSTANT_KEY (JSON_C_OBJECT_KEY_IS_CONSTANT)
#endif

#ifdef __cplusplus
}
#endif
