/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
/*
 * KSVC (Kernel SysCall Virtualization Channel) - Userspace API
 *
 * Shared definitions between kernel module and userspace.
 * This header is safe to include from both kernel and user code.
 *
 * Architecture:
 *   Phase 0: Kernel provides rings + shared page.
 *            Userspace dispatcher handles Tier 1 (io_uring) and Tier 2 (thread pool).
 *   Phase 1: Kernel adds kthread workers for Tier 2 (no privilege transition).
 */
#ifndef _KSVC_UAPI_H
#define _KSVC_UAPI_H

#ifdef __KERNEL__
#include <linux/types.h>
#include <linux/ioctl.h>
#else
#include <stdint.h>
#include <sys/ioctl.h>
typedef uint8_t  __u8;
typedef uint16_t __u16;
typedef uint32_t __u32;
typedef uint64_t __u64;
typedef int32_t  __s32;
typedef int64_t  __s64;
#endif

/* ── Magic numbers ── */
#define KSVC_MAGIC              0x4B535643  /* "KSVC" */
#define KSVC_RING_MAGIC         0x4B52494E  /* "KRIN" */
#define KSVC_SHARED_MAGIC       0x4B534850  /* "KSHP" */
#define KSVC_VERSION            2

/* ── Submission entry ──
 * Written by GVThread into the submit ring.
 * Read by the userspace dispatcher or kernel kthread.
 * 64 bytes = one cache line.
 */
struct ksvc_entry {
    __u64 corr_id;          /* correlation ID (= GVThread ID)        */
    __u32 syscall_nr;       /* __NR_read, __NR_write, etc.           */
    __u32 flags;            /* KSVC_FLAG_*                           */
    __u64 args[6];          /* syscall arguments                     */
} __attribute__((aligned(64)));

/* ── Completion entry ──
 * Written by dispatcher (user or kernel), read by completion handler.
 * 32 bytes.
 */
struct ksvc_completion {
    __u64 corr_id;          /* matches submission corr_id            */
    __s64 result;           /* return value or -errno                */
    __u32 flags;            /* KSVC_COMP_*                           */
    __u32 _pad;
} __attribute__((aligned(32)));

/* ── Ring header ──
 * At the start of each mmap'd ring region.
 * Producer advances tail, consumer advances head.
 * Empty: head == tail.  Full: (tail - head) >= ring_size.
 * 64 bytes (one cache line).
 */
struct ksvc_ring_header {
    __u32 magic;            /* KSVC_RING_MAGIC                       */
    __u32 ring_size;        /* number of entries (power of 2)        */
    __u32 mask;             /* ring_size - 1                         */
    __u32 entry_size;       /* sizeof(ksvc_entry) or sizeof(ksvc_completion) */
    __u64 head;             /* consumer read position                */
    __u64 tail;             /* producer write position               */
    __u64 _reserved[3];
} __attribute__((aligned(64)));

/* ── Shared page ──
 * Kernel-populated, mmap'd read-only into userspace.
 * Tier 0: userspace reads these fields for ~4ns instead of ~200ns syscall.
 *
 * Layout is fixed ABI — new fields append only, never reorder.
 */
struct ksvc_shared_page {
    /* 0x00 */ __u32 magic;
    /* 0x04 */ __u32 version;

    /* 0x08 — Process identity (set once at create time) */
    /* 0x08 */ __s32 pid;
    /* 0x0C */ __s32 tgid;
    /* 0x10 */ __s32 ppid;
    /* 0x14 */ __s32 pgid;
    /* 0x18 */ __s32 sid;
    /* 0x1C */ __s32 _pad_id;

    /* 0x20 — Credentials (set once at create time) */
    /* 0x20 */ __u32 uid;
    /* 0x24 */ __u32 gid;
    /* 0x28 */ __u32 euid;
    /* 0x2C */ __u32 egid;
    /* 0x30 */ __u32 suid;
    /* 0x34 */ __u32 sgid;
    /* 0x38 */ __u32 _pad_cred[2];

    /* 0x40 — System info (set once at create time) */
    /* 0x40 */ char utsname_release[65];    /* uname -r */
    /* 0x81 */ char utsname_nodename[65];   /* hostname  */
    /* 0xC2 */ char utsname_machine[65];    /* arch      */
    /* 0x103 */ __u8 _pad_uts[5];

    /* 0x108 — Resource limits */
    /* 0x108 */ __u64 rlimit_nofile;        /* max open fds   */
    /* 0x110 */ __u64 rlimit_nproc;         /* max processes  */

    /* 0x118 — Reserved for future static fields */
    __u8  _reserved_static[0xE8];           /* pad to 0x200 */

    /* 0x200 — Runtime stats (updated by dispatcher/kthread) */
    /* 0x200 */ __u32 kthread_cpu;          /* CPU id of kthread/dispatcher */
    /* 0x204 */ __u32 worker_state;         /* 0=idle, 1=processing         */
    /* 0x208 */ __u64 entries_processed;    /* monotonic counter            */
    /* 0x210 */ __u64 batches_processed;    /* monotonic counter            */
    /* 0x218 */ __u64 io_uring_inflight;    /* Tier 1 in-flight count       */
    /* 0x220 */ __u64 worker_pool_active;   /* Tier 2 busy workers          */

    /* 0x228 — Ring pointers snapshot (informational) */
    /* 0x228 */ __u64 submit_ring_head;
    /* 0x230 */ __u64 submit_ring_tail;
    /* 0x238 */ __u64 complete_ring_head;
    /* 0x240 */ __u64 complete_ring_tail;

    /* 0x248 — Reserved for future runtime fields */
    __u8  _reserved_runtime[0x38];          /* pad to 0x280 */

    /* 0x280 — Timestamps */
    /* 0x280 */ __u64 clock_monotonic_ns;   /* updated by dispatcher */
    /* 0x288 */ __u64 clock_realtime_ns;    /* updated by dispatcher */
    /* 0x290 */ __u64 boot_time_ns;         /* set once at create    */

    /* 0x298 → 0x1000: expansion space */
};

/* ── ioctl: create instance ── */
struct ksvc_create_params {
    __u32 submit_ring_entries;      /* power of 2, 16..4096 */
    __u32 complete_ring_entries;    /* power of 2, 16..4096 */
    __u32 flags;                    /* KSVC_CREATE_*        */
    __s32 eventfd;                  /* eventfd fd for notifications */
    /* v2 fields: */
    __u32 _reserved[4];
};

/* Submission flags */
#define KSVC_FLAG_LINKED    (1U << 0)
#define KSVC_FLAG_DRAIN     (1U << 1)

/* Completion flags */
#define KSVC_COMP_MORE      (1U << 0)

/* Create flags */
#define KSVC_CREATE_DEFAULT 0

/* ioctl commands */
#define KSVC_IOC_MAGIC  'K'
#define KSVC_IOC_CREATE     _IOWR(KSVC_IOC_MAGIC, 1, struct ksvc_create_params)

/* ── mmap offsets ──
 * Each region is at a page-aligned offset.
 * Userspace: mmap(NULL, size, prot, MAP_SHARED, ksvc_fd, offset)
 *
 * Submit ring:   user writes entries, dispatcher reads.
 * Complete ring: dispatcher writes completions, user reads.
 * Shared page:   kernel writes, user reads (read-only mmap).
 */
#define KSVC_OFF_SUBMIT_RING    0x00000000ULL
#define KSVC_OFF_COMPLETE_RING  0x00100000ULL   /* 1 MiB */
#define KSVC_OFF_SHARED_PAGE    0x00200000ULL   /* 2 MiB */

/* Limits */
#define KSVC_MAX_RING_ENTRIES   4096
#define KSVC_MIN_RING_ENTRIES   16
#define KSVC_MAX_BATCH          64

#endif /* _KSVC_UAPI_H */
