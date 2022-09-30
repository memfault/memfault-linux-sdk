#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Functions to generate Memfault ELF coredump metadata.

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MEMFAULT_CORE_ELF_METADATA_SCHEMA_VERSION_V1 (1)

typedef enum MemfaultCoreElfMetadataKey {
  kMemfaultCoreElfMetadataKey_SchemaVersion = 1,
  kMemfaultCoreElfMetadataKey_LinuxSdkVersion = 2,
  kMemfaultCoreElfMetadataKey_CapturedTime = 3,
  kMemfaultCoreElfMetadataKey_DeviceSerial = 4,
  kMemfaultCoreElfMetadataKey_HardwareVersion = 5,
  kMemfaultCoreElfMetadataKey_SoftwareType = 6,
  kMemfaultCoreElfMetadataKey_SoftwareVersion = 7,
} eMemfaultCoreElfMetadataKey;

typedef struct MemfaultCoreElfMetadata {
  const char *linux_sdk_version;
  uint32_t captured_time_epoch_s;
  const char *device_serial;
  const char *hardware_version;
  const char *software_type;
  const char *software_version;
} sMemfaultCoreElfMetadata;

size_t memfault_core_elf_metadata_note_calculate_size(const sMemfaultCoreElfMetadata *metadata);
bool memfault_core_elf_metadata_note_write(const sMemfaultCoreElfMetadata *metadata,
                                           uint8_t *note_buffer, size_t note_buffer_size);

#ifdef __cplusplus
}
#endif
