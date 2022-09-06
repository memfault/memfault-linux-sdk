//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Testing utilities to convert hex to binary data
//!

#pragma once

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>
#include <stdint.h>

/**
 * @brief Convert a hex string to a binary string
 * @param hex_string The hex string to convert
 * @param[out] out_len The length of the output buffer in bytes
 * @return The heap allocated binary string
 */
uint8_t *memfault_hex2bin(const char *hex_string, size_t *out_len);

/**
 * Convert a hex string to a binary string and write it to a file
 * @param output_path The path at which to write the binary string
 * @param hex_contents The hex string to convert
 */
void memfault_hex2bin_file(const char *output_path, const char *hex_contents);

#ifdef __cplusplus
}
#endif
