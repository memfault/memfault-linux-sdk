//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Utility to process pstore files after booting.

#include "reboot_process_pstore.h"

#include <errno.h>
#include <fts.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

void memfault_reboot_process_pstore_files(char *pstore_dir) {
  // TODO: MFLT-7805 Process last kmsg/console logs
  fprintf(stderr, "reboot:: Cleaning up pstore...\n");

  FTS *fts = NULL;
  char *paths[] = {pstore_dir, NULL};
  if ((fts = fts_open(paths, FTS_PHYSICAL | FTS_NOCHDIR, NULL)) == NULL) {
    fprintf(stderr, "reboot:: fts_open %s\n", strerror(errno));
    return;
  }

  while (true) {
    FTSENT *p = fts_read(fts);
    if (p == NULL) {
      if (errno != 0) {
        fprintf(stderr, "reboot:: fts_read %s\n", strerror(errno));
      }
      goto cleanup;
    }

    switch (p->fts_info) {
      case FTS_F:
      case FTS_SL:
      case FTS_SLNONE:
        fprintf(stderr, "reboot:: unlinking %s...\n", p->fts_path);
        unlink(p->fts_path);
        break;
    }
  }

cleanup:
  fts_close(fts);
}
