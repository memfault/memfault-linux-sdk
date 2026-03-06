//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! shims for older uclibc

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <unistd.h>
#include <bits/wordsize.h>

#ifndef AT_NULL
#define AT_NULL 0
#endif

#ifndef SIZE_MAX
#define SIZE_MAX ((size_t)-1)
#endif

#ifndef PR_GET_NAME
#define PR_GET_NAME 16
#endif

#ifndef PR_SET_NAME
#define PR_SET_NAME 15
#endif

#ifndef O_CLOEXEC
#define O_CLOEXEC 02000000
#endif

// uClibc (especially older versions) may not provide some glibc APIs that
// Rust's std expects on Linux. Provide small shims so we can link.
//
// These are marked weak so a real libc implementation (if present) wins.

// glibc signature: unsigned long getauxval(unsigned long type);
// If /proc/self/auxv exists, we scan it (auxv is a sequence of type/value pairs).
// If the key isn't found, return 0 and set errno=ENOENT like glibc.
__attribute__((weak)) unsigned long getauxval(unsigned long type) {
    int fd = open("/proc/self/auxv", O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        // If we can't open auxv, preserve the errno if it's a real error,
        // otherwise default to ENOENT per glibc behavior.
        if (errno != ENOENT && errno != EACCES) {
            errno = ENOENT;
        }
        return 0;
    }

    // https://github.com/bminor/glibc/blob/master/elf/elf.h#L1173
    // Define auxv entry struct - use pointer size to determine word size
    typedef struct auxv_entry {
        #if __WORDSIZE == 64
                uint64_t a_type;
                uint64_t a_val;
        #elif __WORDSIZE == 32
                uint32_t a_type;
                uint32_t a_val;
        #else
        #error "unknown arch size, please contact Memfault support"
        #endif
    } auxv_entry_t;

    auxv_entry_t entry;

    // Read entries until we find the requested type or reach AT_NULL
    while (1) {
        size_t bytes_read = 0;

        // Handle partial reads and EINTR
        while (bytes_read < sizeof(entry)) {
            ssize_t n = read(fd, ((char*)&entry) + bytes_read,
                           sizeof(entry) - bytes_read);

            if (n < 0) {
                if (errno == EINTR) {
                    continue;
                }
                // Real read error
                close(fd);
                errno = ENOENT;
                return 0;
            }

            if (n == 0) {
                // EOF reached before complete entry - malformed auxv
                close(fd);
                errno = ENOENT;
                return 0;
            }

            bytes_read += n;
        }

        if (entry.a_type == AT_NULL) {
            break;
        }

        if (entry.a_type == type) {
            unsigned long result = entry.a_val;
            close(fd);
            return result;
        }
    }

    // Entry not found
    close(fd);
    errno = ENOENT;
    return 0;
}

// glibc signature: int pthread_setname_np(pthread_t thread, const char *name);
//
// On Linux the thread name limit is 16 bytes including the terminating NUL.
// glibc returns ERANGE when the provided name is too long.
//
// We can only reliably set the name for the calling thread (via prctl).
// For other threads, we return ENOSYS.
__attribute__((weak)) int pthread_setname_np(pthread_t thread, const char *name) {
    if (name == NULL) {
        return EINVAL;
    }

    size_t name_len = strnlen(name, 16);
    if (name_len > 15) {
        return ERANGE;
    }

    if (name[name_len] != '\0') {
        return ERANGE;
    }

    // We can only set the name for the calling thread
    if (!pthread_equal(thread, pthread_self())) {
        return ENOSYS;
    }

    if (prctl(PR_SET_NAME, (unsigned long)name, 0UL, 0UL, 0UL) != 0) {
        int err = errno;
        return err ? err : EINVAL;  // Fallback to EINVAL if errno is somehow 0
    }

    return 0;
}

