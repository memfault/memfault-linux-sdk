#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Parse attributes from the command line into a JSON object.

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct json_object json_object;

/**
 * Converts a list of strings into a JSON object for the PATCH device API.
 *
 * @param argv a list of strings
 * @param argc the number of elements in the list
 * @param json output parameter with a newly allocated json_object (caller responsible to free)
 * @return success status
 */
bool memfaultd_parse_attributes(const char **argv, int argc, json_object **json);

#ifdef __cplusplus
}
#endif
