/* SPDX-License-Identifier: GPL-2.0 */
/*
 * KSVC - Kernel-internal definitions
 * Target: Ubuntu 24.04 / Linux 6.8+
 *
 * Phase 0: rings + shared page only, no kthread.
 * Phase 1: adds kthread worker pool for Tier 2.
 */
#ifndef _KSVC_INTERNAL_H
#define _KSVC_INTERNAL_H

#include <linux/types.h>
#include <linux/fs.h>
#include <linux/miscdevice.h>
#include <linux/mm.h>
#include <linux/sched.h>
#include <linux/eventfd.h>
#include <linux/atomic.h>
#include <linux/ktime.h>

#include "ksvc_uapi.h"

/*
 * Ring buffer — kernel-side bookkeeping.
 * The actual ring data lives in page-allocated memory that gets
 * mmap'd to userspace.
 */
struct ksvc_ring {
    struct page **pages;            /* array of allocated pages          */
    unsigned int nr_pages;          /* total pages (header + data)       */
    void *kaddr;                    /* kernel virtual address            */
    unsigned int nr_entries;        /* number of entries (power of 2)    */
    unsigned int entry_size;        /* bytes per entry                   */
};

/*
 * Per-process KSVC instance.
 * One instance per open(/dev/ksvc) + ioctl(CREATE).
 * Destroyed when the fd is closed.
 */
struct ksvc_instance {
    /* Rings */
    struct ksvc_ring submit;        /* submission ring (user writes)     */
    struct ksvc_ring complete;      /* completion ring (dispatcher writes)*/

    /* Shared page (Tier 0) */
    struct page *shared_page;       /* single page, kernel-populated     */
    struct ksvc_shared_page *shared_kaddr; /* kernel mapping              */

    /* Notification */
    struct eventfd_ctx *eventfd_ctx;/* for signaling userspace           */

    /* State */
    atomic_t created;               /* 1 after successful KSVC_IOC_CREATE */
};

/*
 * Per-fd private data.
 * Stored in file->private_data.
 */
struct ksvc_file_data {
    struct ksvc_instance *inst;     /* NULL until CREATE ioctl           */
};

/* ── ksvc_ring.c ── */
int ksvc_ring_alloc(struct ksvc_ring *ring, unsigned int nr_entries,
                    unsigned int entry_size);
void ksvc_ring_free(struct ksvc_ring *ring);
int ksvc_ring_mmap(struct ksvc_ring *ring, struct vm_area_struct *vma);

/* ── ksvc_shared.c ── */
int ksvc_shared_alloc(struct ksvc_instance *inst);
void ksvc_shared_free(struct ksvc_instance *inst);
void ksvc_shared_populate(struct ksvc_instance *inst);
int ksvc_shared_mmap(struct ksvc_instance *inst, struct vm_area_struct *vma);

/* ── Logging ── */
extern int ksvc_debug;

#define ksvc_dbg(fmt, ...) \
    do { if (ksvc_debug) pr_info("ksvc: " fmt, ##__VA_ARGS__); } while (0)
#define ksvc_info(fmt, ...) pr_info("ksvc: " fmt, ##__VA_ARGS__)
#define ksvc_err(fmt, ...)  pr_err("ksvc: " fmt, ##__VA_ARGS__)

#endif /* _KSVC_INTERNAL_H */
