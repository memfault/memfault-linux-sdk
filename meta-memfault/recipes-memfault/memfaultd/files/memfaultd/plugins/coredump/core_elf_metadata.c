//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Functions to generate Memfault ELF coredump metadata.

#include "core_elf_metadata.h"

#include <stdbool.h>
#include <string.h>

#include "core_elf_note.h"
#include "memfault/util/cbor.h"

static const char s_note_name[] = "Memfault";
static const Elf_Word s_metadata_note_type = 0x4154454d;

static void prv_write_cbor_to_buffer_cb(void *ctx, uint32_t offset, const void *buf,
                                        size_t buf_len) {
  uint8_t *buffer = ctx;
  memcpy(buffer + offset, buf, buf_len);
}

static bool prv_add_schema_version(sMemfaultCborEncoder *encoder) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_SchemaVersion) &&
    memfault_cbor_encode_unsigned_integer(encoder, MEMFAULT_CORE_ELF_METADATA_SCHEMA_VERSION_V1));
}

static bool prv_add_linux_sdk_version(sMemfaultCborEncoder *encoder,
                                      const char *linux_sdk_version) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_LinuxSdkVersion) &&
    memfault_cbor_encode_string(encoder, linux_sdk_version));
}

static bool prv_add_captured_time(sMemfaultCborEncoder *encoder, uint32_t captured_time_epoch_s) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_CapturedTime) &&
    memfault_cbor_encode_unsigned_integer(encoder, captured_time_epoch_s));
}

static bool prv_add_device_serial(sMemfaultCborEncoder *encoder, const char *device_serial) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_DeviceSerial) &&
    memfault_cbor_encode_string(encoder, device_serial));
}

static bool prv_add_hardware_version(sMemfaultCborEncoder *encoder, const char *hardware_version) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_HardwareVersion) &&
    memfault_cbor_encode_string(encoder, hardware_version));
}

static bool prv_add_software_type(sMemfaultCborEncoder *encoder, const char *software_type) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_SoftwareType) &&
    memfault_cbor_encode_string(encoder, software_type));
}

static bool prv_add_software_version(sMemfaultCborEncoder *encoder, const char *software_version) {
  return (
    memfault_cbor_encode_unsigned_integer(encoder, kMemfaultCoreElfMetadataKey_SoftwareVersion) &&
    memfault_cbor_encode_string(encoder, software_version));
}

static bool prv_add_cbor_metadata(sMemfaultCborEncoder *encoder,
                                  const sMemfaultCoreElfMetadata *metadata) {
  return (memfault_cbor_encode_dictionary_begin(encoder, 7) && prv_add_schema_version(encoder) &&
          prv_add_linux_sdk_version(encoder, metadata->linux_sdk_version) &&
          prv_add_captured_time(encoder, metadata->captured_time_epoch_s) &&
          prv_add_device_serial(encoder, metadata->device_serial) &&
          prv_add_hardware_version(encoder, metadata->hardware_version) &&
          prv_add_software_type(encoder, metadata->software_type) &&
          prv_add_software_version(encoder, metadata->software_version));
}

static size_t prv_cbor_calculate_size(const sMemfaultCoreElfMetadata *metadata) {
  sMemfaultCborEncoder encoder;
  memfault_cbor_encoder_size_only_init(&encoder);
  prv_add_cbor_metadata(&encoder, metadata);
  return memfault_cbor_encoder_deinit(&encoder);
}

size_t memfault_core_elf_metadata_note_calculate_size(const sMemfaultCoreElfMetadata *metadata) {
  return memfault_core_elf_note_calculate_size(s_note_name, prv_cbor_calculate_size(metadata));
}

bool memfault_core_elf_metadata_note_write(const sMemfaultCoreElfMetadata *metadata,
                                           uint8_t *note_buffer, size_t note_buffer_size) {
  const size_t description_size = prv_cbor_calculate_size(metadata);
  uint8_t *const description_buffer =
    memfault_core_elf_note_init(note_buffer, s_note_name, description_size, s_metadata_note_type);
  const size_t note_header_size = description_buffer - note_buffer;

  sMemfaultCborEncoder encoder;
  memfault_cbor_encoder_init(&encoder, prv_write_cbor_to_buffer_cb, description_buffer,
                             note_buffer_size - note_header_size);

  return prv_add_cbor_metadata(&encoder, metadata);
}
