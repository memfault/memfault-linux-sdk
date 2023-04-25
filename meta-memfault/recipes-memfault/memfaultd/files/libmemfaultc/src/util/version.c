//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Version information for memfault Linux SDK.
#include <stdio.h>

#ifndef VERSION
  #define VERSION dev
#endif
#ifndef GITCOMMIT
  #define GITCOMMIT unknown
#endif
#ifndef BUILDID
  #define BUILDID unknown
#endif

#define STRINGIZE(x) #x
#define STRINGIZE_VALUE_OF(x) STRINGIZE(x)

const char memfaultd_sdk_version[] = STRINGIZE_VALUE_OF(VERSION);

/**
 * @brief Displays SDK version information
 *
 */
void memfault_version_print_info(void) {
  printf("VERSION=%s\n", STRINGIZE_VALUE_OF(VERSION));
  printf("GIT COMMIT=%s\n", STRINGIZE_VALUE_OF(GITCOMMIT));
  printf("BUILD ID=%s\n", STRINGIZE_VALUE_OF(BUILDID));
}
