//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Simplified definitions for libsystemd. Just enough to get the tests to compile.
//! We only use this file when the system does not have the headers.
//! Implementations will be mocked by the tests.

#include <systemd/sd-bus.h>

sd_bus_error SD_BUS_ERROR_NULL = {.name = 0};
