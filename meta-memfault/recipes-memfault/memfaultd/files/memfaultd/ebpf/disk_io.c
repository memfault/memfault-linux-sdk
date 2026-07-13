//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Memfault disk I/O ebpf program

#include <linux/types.h>
#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>

#include "vmlinux.h"

char LICENSE[] SEC("license") = "Dual BSD/GPL";

#define DISK_OP_READ  0
#define DISK_OP_WRITE 1

// Upper bound on the number of unique (tgid, dev, op) tuples we can hold
// between userspace drains. Exceeding this increments DISK_IO_DROPS.
#define DISK_IO_MAX_ENTRIES 8192

struct disk_io_key {
    __u32 tgid;
    __u32 dev;
    __u32 op;
};

// Per-CPU keeps each CPU's counter independent, avoiding cross-CPU update
// races without needing atomics. Userspace sums across CPUs at read time.
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_HASH);
    __type(key, struct disk_io_key);
    __type(value, __u64);
    __uint(max_entries, DISK_IO_MAX_ENTRIES);
} DISK_IO_STATS SEC(".maps");

// Cumulative count of bio events that couldn't be recorded because
// DISK_IO_STATS was full. Userspace computes deltas across reads.
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __type(key, __u32);
    __type(value, __u64);
    __uint(max_entries, 1);
} DISK_IO_DROPS SEC(".maps");

static __always_inline void record_drop(void)
{
    __u32 zero = 0;
    __u64 *counter = bpf_map_lookup_elem(&DISK_IO_DROPS, &zero);
    if (counter) {
        *counter += 1;
    }
}

// rwbs is a short string like "R", "W", "RA", "WS", "FW". We only care about
// read vs. write; flush/discard/other get skipped.
static __always_inline int classify_op(const char *rwbs)
{
    #pragma unroll
    for (int i = 0; i < 8; i++) {
        char c = rwbs[i];
        if (c == '\0') {
            break;
        }
        if (c == 'W') {
            return DISK_OP_WRITE;
        }
        if (c == 'R') {
            return DISK_OP_READ;
        }
    }
    return -1;
}

SEC("tracepoint/block/block_io_start")
int handle_block_io_start(struct trace_event_raw_block_rq *ctx)
{
    char rwbs[8];
    bpf_core_read(rwbs, sizeof(rwbs), &ctx->rwbs);

    int op = classify_op(rwbs);
    if (op < 0) {
        return 0;
    }

    __u64 bytes = BPF_CORE_READ(ctx, bytes);
    if (bytes == 0) {
        return 0;
    }

    struct disk_io_key key = {
        .tgid = bpf_get_current_pid_tgid() >> 32,
        .dev = BPF_CORE_READ(ctx, dev),
        .op = (__u32)op,
    };

    __u64 *counter = bpf_map_lookup_elem(&DISK_IO_STATS, &key);
    if (counter) {
        *counter += bytes;
        return 0;
    }

    if (bpf_map_update_elem(&DISK_IO_STATS, &key, &bytes, BPF_NOEXIST) != 0) {
        record_drop();
    }
    return 0;
}
