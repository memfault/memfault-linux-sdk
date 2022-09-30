//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! ELF coredump reader

#include "core_elf_reader.h"

#include <assert.h>
#include <errno.h>
#include <malloc.h>
#include <string.h>
#include <unistd.h>

#include "memfault/util/string.h"

bool memfault_core_elf_reader_is_valid_core_elf(const void *elf_buffer, size_t buffer_size) {
  if (elf_buffer == NULL || buffer_size < sizeof(Elf_Ehdr)) {
    return false;
  }
  const Elf_Ehdr *const header = (const Elf_Ehdr *)elf_buffer;

  return (header->e_ident[0] == ELFMAG0 && header->e_ident[1] == ELFMAG1 &&
          header->e_ident[2] == ELFMAG2 && header->e_ident[3] == ELFMAG3 &&
          header->e_ident[4] == ELFCLASS && header->e_version == EV_CURRENT &&
          header->e_ehsize == sizeof(Elf_Ehdr) && header->e_phentsize == sizeof(Elf_Phdr) &&
          header->e_type == ET_CORE);
}

void memfault_core_elf_reader_init(sMemfaultCoreElfReader *reader, const sMemfaultCoreElfReadIO *io,
                                   sMemfaultCoreElfReaderHandler *handler) {
  *reader = (sMemfaultCoreElfReader){
    .handler = handler,
    .io = io,
  };
}

static void prv_action_done(sMemfaultCoreElfReader *reader) {
  reader->handler->handle_done(reader);
  free(reader->segments);
  reader->action = NULL;
}

static size_t prv_read_all(sMemfaultCoreElfReader *reader, uint8_t *buffer, size_t size) {
  if (size == 0) {
    return 0;
  }
  size_t size_remaining = size;
  uint8_t *cursor = buffer;
  while (size_remaining > 0) {
    const ssize_t read_size = reader->io->read(reader->io, cursor, size_remaining);
    if (read_size == -1) {
      if (errno == EINTR) {
        continue;
      }
      char *warning = NULL;
      memfault_asprintf(&warning, "read() failure: %s", strerror(errno));
      reader->handler->handle_warning(reader, warning);
      break;
    }
    reader->stream_pos += read_size;
    size_remaining -= read_size;
    if (cursor != NULL) {
      cursor += read_size;
    }
    if (read_size == 0) {
      break;
    }
  }
  return size - size_remaining;
}

static void prv_action_read_segment_headers(sMemfaultCoreElfReader *reader) {
  const size_t total_segments_size = reader->elf_header.e_phnum * sizeof(Elf_Phdr);
  const size_t read_size = prv_read_all(reader, (uint8_t *)reader->segments, total_segments_size);
  if (read_size == total_segments_size) {
    reader->handler->handle_segments(reader, reader->segments, reader->elf_header.e_phnum);
  } else {
    reader->handler->handle_warning(reader,
                                    strdup("Unexpected short read while reading segment headers"));
  }
  reader->action = prv_action_done;
}

static void prv_action_prepare_segment_headers(sMemfaultCoreElfReader *reader) {
  assert(reader->segments == NULL);
  const size_t total_segments_size = reader->elf_header.e_phnum * sizeof(Elf_Phdr);
  if (total_segments_size > 0) {
    Elf_Phdr *segments = malloc(total_segments_size);
    if (segments == NULL) {
      char *warning = NULL;
      memfault_asprintf(&warning, "Not enough memory for %d segment headers",
                        reader->elf_header.e_phnum);
      reader->handler->handle_warning(reader, warning);
      reader->action = prv_action_done;
      return;
    }
    reader->segments = segments;
  }
  reader->action = prv_action_read_segment_headers;
}

static void prv_action_skip_to_segment_headers(sMemfaultCoreElfReader *reader) {
  assert(reader->elf_header.e_phoff >= reader->stream_pos);
  const size_t skip_size = reader->elf_header.e_phoff - reader->stream_pos;
  const size_t read_size = prv_read_all(reader, NULL, skip_size);
  if (read_size < skip_size) {
    reader->handler->handle_warning(reader, strdup("Unexpected short read while skipping"));
    reader->action = prv_action_done;
    return;
  }
  reader->action = prv_action_prepare_segment_headers;
}

static void prv_action_read_elf_header(sMemfaultCoreElfReader *reader) {
  const size_t read_size =
    prv_read_all(reader, (uint8_t *)&reader->elf_header, sizeof(reader->elf_header));

  assert(read_size <= sizeof(reader->elf_header));
  if (read_size < sizeof(reader->elf_header)) {
    reader->handler->handle_warning(reader,
                                    strdup("Unexpected short read while reading ELF header"));
    reader->action = prv_action_done;
    return;
  }

  if (!memfault_core_elf_reader_is_valid_core_elf(&reader->elf_header,
                                                  sizeof(reader->elf_header))) {
    reader->handler->handle_warning(reader, strdup("Not an ELF coredump"));
    reader->action = prv_action_done;
    return;
  }

  reader->handler->handle_elf_header(reader, &reader->elf_header);

  if (reader->elf_header.e_phoff == reader->stream_pos) {
    reader->action = prv_action_prepare_segment_headers;
  } else if (reader->elf_header.e_phoff > reader->stream_pos) {
    reader->handler->handle_warning(reader,
                                    strdup("Ignoring data between header and segment table"));
    reader->action = prv_action_skip_to_segment_headers;
  } else {
    reader->handler->handle_warning(reader, strdup("Unexpected segment table offset"));
    reader->action = prv_action_done;
  }
}

bool memfault_core_elf_reader_read_all(sMemfaultCoreElfReader *reader) {
  if (reader->action != NULL) {
    return false;
  }
  reader->action = prv_action_read_elf_header;
  while (reader->action != NULL) {
    reader->action(reader);
  }
  return true;
}

size_t memfault_core_elf_reader_read_segment_data(sMemfaultCoreElfReader *reader, size_t at_pos,
                                                  uint8_t *buffer, size_t buffer_size) {
  // Note: this function should only be called from the handle_segments callback.
  assert(reader->action == prv_action_read_segment_headers);
  if (at_pos < reader->stream_pos) {
    // Already past the requested stream_pos
    return 0;
  }

  // Skip to at_pos:
  const size_t skip_size = at_pos - reader->stream_pos;
  if (skip_size > 0) {
    const size_t read_size = prv_read_all(reader, NULL, skip_size);
    if (read_size < skip_size) {
      return 0;  // EOF
    }
  }

  return prv_read_all(reader, buffer, buffer_size);
}

static ssize_t prv_fio_read(const struct MemfaultCoreElfReadIO *io, void *buffer,
                            size_t buffer_size) {
  sMemfaultCoreElfReadFileIO *fio = (sMemfaultCoreElfReadFileIO *)io;
  return read(fio->fd, buffer, buffer_size);
}

void memfault_core_elf_read_file_io_init(sMemfaultCoreElfReadFileIO *fio, int fd) {
  *fio = (sMemfaultCoreElfReadFileIO){
    .io =
      {
        .read = prv_fio_read,
      },
    .fd = fd,
  };
}
