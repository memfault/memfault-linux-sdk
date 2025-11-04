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

struct evt {
    __u32 pid;
    __u32 bytes;
    __u32 dev;     // dev_t truncated to 32 bits for simplicity
    char rwbs[8];
};

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __type(key, int);
    __uint(value_size, sizeof(__u32));
    __uint(max_entries, 1024);
} DISK_EVENTS SEC(".maps");

// tracepoint arguments layout (from /sys/kernel/debug/tracing/events/block/block_io_start/format)
struct block_io_start_args {
    __u16 common_type;           // offset:0,  size:2, signed:0
    __u8  common_flags;          // offset:2,  size:1, signed:0
    __u8  common_preempt_count;  // offset:3,  size:1, signed:0
    __s32 common_pid;            // offset:4,  size:4, signed:1

    __u32 dev;                   // offset:8,  size:4, signed:0 (dev_t)
    __u32 __pad1;                // offset:12, size:4, padding for alignment
    __u64 sector;                // offset:16, size:8, signed:0 (sector_t)
    __u32 nr_sector;             // offset:24, size:4, signed:0
    __u32 bytes;                 // offset:28, size:4, signed:0
#if __KERNEL >= 611
    __u16 ioprio;                // offset:32, size:2, signed:0
#endif
    char  rwbs[8];               // offset:34, size:8, signed:0
    char  comm[16];              // offset:42, size:16, signed:0
    __u32 cmd_data_loc;          // offset:60, size:4, signed:0 (__data_loc)
};

SEC("tracepoint/block/block_io_start")
int handle_block_io_start(struct block_io_start_args *ctx)
{
    struct evt e = {};

    e.pid = bpf_get_current_pid_tgid() >> 32;
    e.dev = ctx->dev;
    e.bytes = ctx->bytes;

    __builtin_memcpy(e.rwbs, ctx->rwbs, sizeof(e.rwbs));

    // submit via perf event
    bpf_perf_event_output(ctx, &DISK_EVENTS, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}
