//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultd device settings API definition

#ifndef __DEVICE_SETTINGS_H
#define __DEVICE_SETTINGS_H

#include "memfaultd.h"

#ifdef __cplusplus
extern "C" {
#endif

sMemfaultdDeviceSettings *memfaultd_device_settings_init(void);
void memfaultd_device_settings_destroy(sMemfaultdDeviceSettings *handle);

#ifdef __cplusplus
}
#endif

#endif
