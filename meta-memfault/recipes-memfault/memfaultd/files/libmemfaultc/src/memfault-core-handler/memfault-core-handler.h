#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfault-core-handler program that accepts coredumps from the Linux kernel.

#include <stdlib.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum MemfaultCoreHandlerStatus {
  kMemfaultCoreHandlerStatus_Ok = EXIT_SUCCESS,
  kMemfaultCoreHandlerStatus_InvalidArguments = 1,
  kMemfaultCoreHandlerStatus_InvalidConfiguration = 2,
  kMemfaultCoreHandlerStatus_OOM = 3,
  kMemfaultCoreHandlerStatus_DiskQuotaExceeded = 4,
  kMemfaultCoreHandlerStatus_DeviceSettingsFailure = 5,
} eMemfaultCoreHandlerStatus;

int memfault_core_handler_main(int argc, char *argv[]);

#ifdef __cplusplus
}
#endif
