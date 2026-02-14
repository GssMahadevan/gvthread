// SPDX-License-Identifier: GPL-2.0
/*
 * KSVC - Ring buffer allocation and mmap
 *
 * Rings are allocated as an array of individual pages (not compound pages).
 * This makes mmap via vm_insert_page() straightforward and avoids
 * fragmentation issues with large contiguous allocations.
 *
 * Layout in memory:
 *   Page 0:      Ring header (64 bytes used, rest zero)
 *   Pages 1..N:  Entry array
 *
 * The header page and data pages are mapped contiguously into userspace.
 */

#include <linux/slab.h>
#include <linux/mm.h>
#include <linux/string.h>

#include "ksvc_internal.h"

/*
 * Calculate number of pages needed for nr_entries of entry_size bytes,
 * plus one page for the ring header.
 */
static unsigned int ring_pages_needed(unsigned int nr_entries,
                                      unsigned int entry_size)
{
    unsigned long data_bytes = (unsigned long)nr_entries * entry_size;
    unsigned int data_pages = DIV_ROUND_UP(data_bytes, PAGE_SIZE);
    return 1 + data_pages;  /* 1 header page + data pages */
}

int ksvc_ring_alloc(struct ksvc_ring *ring, unsigned int nr_entries,
                    unsigned int entry_size)
{
    unsigned int nr_pages, i;
    struct ksvc_ring_header *hdr;

    nr_pages = ring_pages_needed(nr_entries, entry_size);

    ksvc_dbg("ring_alloc: entries=%u entry_size=%u pages=%u\n",
             nr_entries, entry_size, nr_pages);

    /* Allocate page pointer array */
    ring->pages = kcalloc(nr_pages, sizeof(struct page *), GFP_KERNEL);
    if (!ring->pages)
        return -ENOMEM;

    /* Allocate individual pages */
    for (i = 0; i < nr_pages; i++) {
        ring->pages[i] = alloc_page(GFP_KERNEL | __GFP_ZERO);
        if (!ring->pages[i]) {
            ksvc_err("ring_alloc: page alloc failed at page %u/%u\n",
                     i, nr_pages);
            goto err_free_pages;
        }
    }

    ring->nr_pages = nr_pages;
    ring->nr_entries = nr_entries;
    ring->entry_size = entry_size;

    /* Map the header page into kernel address space for initialization */
    ring->kaddr = page_address(ring->pages[0]);
    if (!ring->kaddr) {
        /* page_address can return NULL for highmem pages on 32-bit.
         * On 64-bit this should never happen. */
        ksvc_err("ring_alloc: page_address returned NULL\n");
        goto err_free_pages;
    }

    /* Initialize the ring header */
    hdr = (struct ksvc_ring_header *)ring->kaddr;
    hdr->magic = KSVC_RING_MAGIC;
    hdr->ring_size = nr_entries;
    hdr->mask = nr_entries - 1;
    hdr->entry_size = entry_size;
    hdr->head = 0;
    hdr->tail = 0;

    ksvc_dbg("ring_alloc: success, %u pages, header at %p\n",
             nr_pages, ring->kaddr);
    return 0;

err_free_pages:
    while (i-- > 0)
        __free_page(ring->pages[i]);
    kfree(ring->pages);
    ring->pages = NULL;
    return -ENOMEM;
}

void ksvc_ring_free(struct ksvc_ring *ring)
{
    unsigned int i;

    if (!ring->pages)
        return;

    for (i = 0; i < ring->nr_pages; i++) {
        if (ring->pages[i])
            __free_page(ring->pages[i]);
    }
    kfree(ring->pages);

    ring->pages = NULL;
    ring->kaddr = NULL;
    ring->nr_pages = 0;
}

/*
 * mmap a ring into userspace.
 *
 * Uses vm_insert_page() per page — safe, portable, works with
 * both MAP_SHARED and MAP_PRIVATE, and handles refcounting correctly.
 *
 * No VM_PFNMAP needed. The pages are normal kernel-allocated pages
 * with valid struct page *, so vm_insert_page() is the right API.
 */
int ksvc_ring_mmap(struct ksvc_ring *ring, struct vm_area_struct *vma)
{
    unsigned long size = vma->vm_end - vma->vm_start;
    unsigned long expected = (unsigned long)ring->nr_pages << PAGE_SHIFT;
    unsigned long addr;
    unsigned int i;
    int ret;

    if (size != expected) {
        ksvc_err("ring_mmap: size mismatch: got %lu, expected %lu\n",
                 size, expected);
        return -EINVAL;
    }

    /* Don't allow fork to inherit these mappings */
    vm_flags_set(vma, VM_DONTCOPY | VM_DONTEXPAND);

    /* Insert each page */
    addr = vma->vm_start;
    for (i = 0; i < ring->nr_pages; i++) {
        ret = vm_insert_page(vma, addr, ring->pages[i]);
        if (ret) {
            ksvc_err("ring_mmap: vm_insert_page failed at page %u: %d\n",
                     i, ret);
            /* Pages already inserted will be unmapped on vma destruction */
            return ret;
        }
        addr += PAGE_SIZE;
    }

    ksvc_dbg("ring_mmap: mapped %u pages at %lx-%lx\n",
             ring->nr_pages, vma->vm_start, vma->vm_end);
    return 0;
}
