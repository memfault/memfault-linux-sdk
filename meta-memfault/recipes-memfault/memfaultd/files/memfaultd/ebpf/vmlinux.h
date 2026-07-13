//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! vmlinux.h stubs for CORE

#ifndef __VMLINUX_H__
#define __VMLINUX_H__

#include <linux/types.h>

/*
 * Minimal CO-RE declarations for Memfault's eBPF programs.
 *
 * Only the kernel types accessed by our programs are declared. Field offsets
 * are relocated at load time from the running kernel's BTF, so these structs
 * only need matching field names, not matching layouts.
 */

#pragma clang attribute push(__attribute__((preserve_access_index)), apply_to = record)

struct trace_event_raw_block_rq {
    __u32 dev;
    __u32 bytes;
    char rwbs[8];
};

#pragma clang attribute pop

#endif /* __VMLINUX_H__ */
