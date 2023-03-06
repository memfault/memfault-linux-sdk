//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include "crash.h"

#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/types.h>
#include <unistd.h>

#include "memfault/util/ipc.h"

void memfault_trigger_crash(eErrorType e) {
  int pid = fork();

  // Child crashes itself
  if (pid == 0) {
    switch (e) {
      case eErrorTypeSegFault:
        (*((int *)0))++;
        break;
      case eErrorTypeFPException: {
        // Triggers an illegal instruction on x86 - Floating Point Error on ARM
        printf("%i", 42 / (pid));
        break;
      }
      default:
        fprintf(stderr, "FIXME: Error type %i not implemented.", e);
    }
    fprintf(stderr, "failed to crash?\n");
    exit(-1);
  }

  // Parent continues.
}
