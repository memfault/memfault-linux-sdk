//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include <stdio.h>

void memfault_trigger_fp_exception(void) {
  int divisor = 0;
  // Triggers an illegal instruction on x86 - Floating Point Error on ARM
  printf("%i", 42 / divisor);
}
