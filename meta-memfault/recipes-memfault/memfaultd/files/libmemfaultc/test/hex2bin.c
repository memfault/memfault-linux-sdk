//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Testing utilities to convert hex to binary data
//!

#include "hex2bin.h"

#include <assert.h>
#include <fcntl.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

uint8_t *memfault_hex2bin(const char *hex_string, size_t *out_len) {
  const size_t hex_len = strlen(hex_string);
  assert(hex_len % 2 == 0);
  uint8_t *buffer = malloc(hex_len / 2);
  for (size_t i = 0; i < hex_len; i += 2) {
    char hex_byte[3] = {hex_string[i], hex_string[i + 1], '\0'};
    buffer[i / 2] = strtol(hex_byte, NULL, 16);
  }
  if (out_len) {
    *out_len = hex_len / 2;
  }
  return buffer;
}

void memfault_hex2bin_file(const char *output_path, const char *hex_contents) {
  size_t contents_size;
  uint8_t *const contents = memfault_hex2bin(hex_contents, &contents_size);
  const int fd = open(output_path, O_RDWR | O_CREAT, S_IRUSR | S_IWUSR);
  write(fd, contents, contents_size);
  close(fd);
  free(contents);
}
