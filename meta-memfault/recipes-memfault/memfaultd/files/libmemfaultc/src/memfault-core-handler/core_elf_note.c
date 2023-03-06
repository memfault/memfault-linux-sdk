//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! ELF note utilities

#include "core_elf_note.h"

#include <string.h>

#include "memfault/core/math.h"

static size_t prv_calc_owner_name_size(const char *owner_name) {
  // "If no name is present, namesz contains 0."
  const size_t owner_name_strlen = owner_name == NULL ? 0 : strlen(owner_name);
  const size_t owner_name_size =
    owner_name_strlen > 0 ? owner_name_strlen + 1 : 0;  // +1 for NUL terminator
  return owner_name_size;
}

size_t memfault_core_elf_note_calculate_size(const char *owner_name, size_t description_size) {
  const size_t owner_name_size = prv_calc_owner_name_size(owner_name);
  return sizeof(Elf_Nhdr) + MEMFAULT_ALIGN_UP(owner_name_size, 4) +
         MEMFAULT_ALIGN_UP(description_size, 4);
}

uint8_t *memfault_core_elf_note_init(void *out_buffer, const char *owner_name,
                                     size_t description_size, Elf_Word n_type) {
  const size_t owner_name_size = prv_calc_owner_name_size(owner_name);
  *(Elf_Nhdr *)out_buffer = (Elf_Nhdr){
    .n_namesz = owner_name_size,
    .n_descsz = description_size,
    .n_type = n_type,
  };
  uint8_t *const payload = ((uint8_t *)out_buffer) + sizeof(Elf_Nhdr);
  const size_t owner_name_and_padding_size = MEMFAULT_ALIGN_UP(owner_name_size, 4);
  memset(payload, 0, owner_name_and_padding_size + MEMFAULT_ALIGN_UP(description_size, 4));
  memcpy(payload, owner_name, owner_name_size);
  return (payload + owner_name_and_padding_size);
}
