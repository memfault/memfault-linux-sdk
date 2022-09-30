#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! String utilities.

#include "memfault/core/compiler.h"

MEMFAULT_PRINTF_LIKE_FUNC(2, 3)
int memfault_asprintf(char **restrict strp, const char *fmt, ...);
