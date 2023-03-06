//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Logging utilities.

#include "memfault/util/logging.h"

#ifdef HAVE_SYSTEMD
  #include <systemd/sd-journal.h>
#endif

#include <stdarg.h>
#include <stdio.h>

static eMemfaultdLogLevel s_min_log_level = kMemfaultdLogLevel_Warning;
static eMemfaultdLogDestination s_log_destination = kMemfaultdLogDestination_Stderr;

void memfaultd_log_configure(eMemfaultdLogLevel min_level, eMemfaultdLogDestination destination) {
  s_min_log_level = min_level;
  s_log_destination = destination;
}

#ifdef HAVE_SYSTEMD
static int prv_level_to_systemd_journal_level(eMemfaultdLogLevel level) {
  switch (level) {
    case kMemfaultdLogLevel_Debug:
      return LOG_DEBUG;
    case kMemfaultdLogLevel_Info:
      return LOG_INFO;
    case kMemfaultdLogLevel_Warning:
      return LOG_WARNING;
    case kMemfaultdLogLevel_Error:
    default:
      return LOG_ERR;
  }
}
#endif

void memfaultd_log(eMemfaultdLogLevel level, const char *fmt, ...) {
  if (level < s_min_log_level) {
    return;
  }

  va_list(args);
  va_start(args, fmt);

  switch (s_log_destination) {
#ifdef HAVE_SYSTEMD
    case kMemfaultdLogDestination_SystemdJournal:
      sd_journal_printv(prv_level_to_systemd_journal_level(level), fmt, args);
      break;
#endif
    case kMemfaultdLogDestination_Stderr:
    default:
      vfprintf(stderr, fmt, args);
      fprintf(stderr, "\n");
      break;
  }

  va_end(args);
}
