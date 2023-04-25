#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Triggers a floating point exception by dividing by zero.
 *
 * Note that this still needs to be in C as Rust protects against divide-by-zero.
 * at runtime.
 */
void memfault_trigger_fp_exception(void);

#ifdef __cplusplus
}
#endif
