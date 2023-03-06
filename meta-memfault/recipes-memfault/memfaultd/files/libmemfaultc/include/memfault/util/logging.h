#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Logging utilities.

#include "memfault/core/compiler.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum MemfaultdLogLevel {
  kMemfaultdLogLevel_Debug = 0,
  kMemfaultdLogLevel_Info,
  kMemfaultdLogLevel_Warning,
  kMemfaultdLogLevel_Error,
} eMemfaultdLogLevel;

typedef enum MemfaultdLogDestination {
  kMemfaultdLogDestination_Stderr,
  kMemfaultdLogDestination_SystemdJournal,
} eMemfaultdLogDestination;

void memfaultd_log_configure(eMemfaultdLogLevel min_level, eMemfaultdLogDestination destination);

MEMFAULT_PRINTF_LIKE_FUNC(2, 3)
void memfaultd_log(eMemfaultdLogLevel level, const char *fmt, ...);

#ifdef __cplusplus
}
#endif
