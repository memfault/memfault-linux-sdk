#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! ELF note utilities

#include <stddef.h>
#include <stdint.h>

#include "core_elf.h"

#ifdef __cplusplus
extern "C" {
#endif

size_t memfault_core_elf_note_calculate_size(const char *owner_name, size_t description_size);

uint8_t *memfault_core_elf_note_init(void *out_buffer, const char *owner_name,
                                     size_t description_size, Elf_Word n_type);

#ifdef __cplusplus
}
#endif
