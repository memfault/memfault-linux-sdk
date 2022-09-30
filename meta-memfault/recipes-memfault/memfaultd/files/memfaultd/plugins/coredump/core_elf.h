#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! ELF common definitions

#include <elf.h>
#include <limits.h>
#include <link.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef ElfW(Ehdr) Elf_Ehdr;
typedef ElfW(Nhdr) Elf_Nhdr;
typedef ElfW(Phdr) Elf_Phdr;
typedef ElfW(Shdr) Elf_Shdr;
typedef ElfW(Word) Elf_Word;
typedef ElfW(Half) Elf_Half;
typedef ElfW(Addr) Elf_Addr;
typedef ElfW(Off) Elf_Off;

#if ULONG_MAX == 0xffffffff
  #define ELFCLASS (ELFCLASS32)
#elif ULONG_MAX == 0xffffffffffffffff
  #define ELFCLASS (ELFCLASS64)
#else
  #error "Unsupported word size"
#endif

#if !defined(__BYTE_ORDER__) || !defined(__ORDER_BIG_ENDIAN__) || !defined(__ORDER_LITTLE_ENDIAN__)
  #error "__BYTE_ORDER__ is not defined"
#endif

#if __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
  #define ELFDATA (ELFDATA2LSB)
#elif __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
  #define ELFDATA (ELFDATA2MSB)
#else
  #error "Unsupported byte order"
#endif

#ifdef __cplusplus
}
#endif
