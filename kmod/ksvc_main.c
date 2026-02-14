// SPDX-License-Identifier: GPL-2.0
/*
 * KSVC - Main module file
 *
 * /dev/ksvc miscdevice with:
 *   open/release:  per-fd state
 *   ioctl CREATE:  allocate rings + shared page
 *   mmap:          map rings and shared page to userspace
 *
 * Target: Ubuntu 24.04 / Linux 6.8+
 * Phase 0: rings + shared page only, no kthread.
 */

#include <linux/module.h>
#include <linux/miscdevice.h>
#include <linux/fs.h>
#include <linux/slab.h>
#include <linux/uaccess.h>
#include <linux/eventfd.h>

#include "ksvc_internal.h"

MODULE_LICENSE("GPL");
MODULE_AUTHOR("GssMahadevan");
MODULE_DESCRIPTION("KSVC - Kernel SysCall Virtualization Channel");
MODULE_VERSION("0.2.0");

int ksvc_debug = 0;
module_param(ksvc_debug, int, 0644);
MODULE_PARM_DESC(ksvc_debug, "Enable debug logging (default: 0)");

/* ── Instance lifecycle ── */

static struct ksvc_instance *ksvc_instance_create(void)
{
    struct ksvc_instance *inst;

    inst = kzalloc(sizeof(*inst), GFP_KERNEL);
    if (!inst)
        return ERR_PTR(-ENOMEM);

    atomic_set(&inst->created, 0);
    return inst;
}

static void ksvc_instance_destroy(struct ksvc_instance *inst)
{
    if (!inst)
        return;

    if (inst->eventfd_ctx)
        eventfd_ctx_put(inst->eventfd_ctx);

    ksvc_ring_free(&inst->submit);
    ksvc_ring_free(&inst->complete);
    ksvc_shared_free(inst);

    kfree(inst);
}

/* ── ioctl: CREATE ── */

static long ksvc_ioctl_create(struct ksvc_instance *inst,
                              unsigned long arg)
{
    struct ksvc_create_params params;
    int ret;

    /* Only one CREATE per instance */
    if (atomic_read(&inst->created)) {
        ksvc_err("instance already created\n");
        return -EBUSY;
    }

    if (copy_from_user(&params, (void __user *)arg, sizeof(params)))
        return -EFAULT;

    /* Validate ring sizes */
    if (params.submit_ring_entries < KSVC_MIN_RING_ENTRIES ||
        params.submit_ring_entries > KSVC_MAX_RING_ENTRIES ||
        !is_power_of_2(params.submit_ring_entries)) {
        ksvc_err("invalid submit ring size %u\n", params.submit_ring_entries);
        return -EINVAL;
    }

    if (params.complete_ring_entries < KSVC_MIN_RING_ENTRIES ||
        params.complete_ring_entries > KSVC_MAX_RING_ENTRIES ||
        !is_power_of_2(params.complete_ring_entries)) {
        ksvc_err("invalid complete ring size %u\n", params.complete_ring_entries);
        return -EINVAL;
    }

    /* Allocate submit ring */
    ret = ksvc_ring_alloc(&inst->submit, params.submit_ring_entries,
                          sizeof(struct ksvc_entry));
    if (ret) {
        ksvc_err("submit ring alloc failed: %d\n", ret);
        return ret;
    }

    /* Allocate completion ring */
    ret = ksvc_ring_alloc(&inst->complete, params.complete_ring_entries,
                          sizeof(struct ksvc_completion));
    if (ret) {
        ksvc_err("complete ring alloc failed: %d\n", ret);
        goto err_free_submit;
    }

    /* Allocate and populate shared page */
    ret = ksvc_shared_alloc(inst);
    if (ret) {
        ksvc_err("shared page alloc failed: %d\n", ret);
        goto err_free_complete;
    }
    ksvc_shared_populate(inst);

    /* Acquire eventfd context */
    if (params.eventfd >= 0) {
        inst->eventfd_ctx = eventfd_ctx_fdget(params.eventfd);
        if (IS_ERR(inst->eventfd_ctx)) {
            ret = PTR_ERR(inst->eventfd_ctx);
            inst->eventfd_ctx = NULL;
            ksvc_err("eventfd_ctx_fdget failed: %d\n", ret);
            goto err_free_shared;
        }
    }

    /* Write back any output params */
    if (copy_to_user((void __user *)arg, &params, sizeof(params))) {
        ret = -EFAULT;
        goto err_free_eventfd;
    }

    atomic_set(&inst->created, 1);

    ksvc_info("instance created: submit=%u complete=%u eventfd=%d\n",
              params.submit_ring_entries, params.complete_ring_entries,
              params.eventfd);
    return 0;

err_free_eventfd:
    if (inst->eventfd_ctx) {
        eventfd_ctx_put(inst->eventfd_ctx);
        inst->eventfd_ctx = NULL;
    }
err_free_shared:
    ksvc_shared_free(inst);
err_free_complete:
    ksvc_ring_free(&inst->complete);
err_free_submit:
    ksvc_ring_free(&inst->submit);
    return ret;
}

/* ── file_operations ── */

static int ksvc_open(struct inode *inode, struct file *file)
{
    struct ksvc_file_data *fdata;

    fdata = kzalloc(sizeof(*fdata), GFP_KERNEL);
    if (!fdata)
        return -ENOMEM;

    fdata->inst = ksvc_instance_create();
    if (IS_ERR(fdata->inst)) {
        int ret = PTR_ERR(fdata->inst);
        kfree(fdata);
        return ret;
    }

    file->private_data = fdata;
    ksvc_dbg("opened by pid %d\n", current->pid);
    return 0;
}

static int ksvc_release(struct inode *inode, struct file *file)
{
    struct ksvc_file_data *fdata = file->private_data;

    if (fdata) {
        ksvc_dbg("released by pid %d\n", current->pid);
        ksvc_instance_destroy(fdata->inst);
        kfree(fdata);
    }
    return 0;
}

static long ksvc_ioctl(struct file *file, unsigned int cmd, unsigned long arg)
{
    struct ksvc_file_data *fdata = file->private_data;

    if (!fdata || !fdata->inst)
        return -EINVAL;

    switch (cmd) {
    case KSVC_IOC_CREATE:
        return ksvc_ioctl_create(fdata->inst, arg);
    default:
        return -ENOTTY;
    }
}

static int ksvc_mmap(struct file *file, struct vm_area_struct *vma)
{
    struct ksvc_file_data *fdata = file->private_data;
    struct ksvc_instance *inst;
    unsigned long offset;

    if (!fdata || !fdata->inst)
        return -EINVAL;

    inst = fdata->inst;
    if (!atomic_read(&inst->created))
        return -EINVAL;

    offset = vma->vm_pgoff << PAGE_SHIFT;

    ksvc_dbg("mmap: offset=0x%lx size=%lu\n", offset,
             vma->vm_end - vma->vm_start);

    switch (offset) {
    case KSVC_OFF_SUBMIT_RING:
        /* Submit ring: user writes entries, dispatcher reads.
         * User needs read+write. */
        return ksvc_ring_mmap(&inst->submit, vma);

    case KSVC_OFF_COMPLETE_RING:
        /* Complete ring: dispatcher writes, user reads.
         * User needs read+write (to advance head). */
        return ksvc_ring_mmap(&inst->complete, vma);

    case KSVC_OFF_SHARED_PAGE:
        /* Shared page: kernel writes, user reads only. */
        return ksvc_shared_mmap(inst, vma);

    default:
        ksvc_err("mmap: unknown offset 0x%lx\n", offset);
        return -EINVAL;
    }
}

static const struct file_operations ksvc_fops = {
    .owner          = THIS_MODULE,
    .open           = ksvc_open,
    .release        = ksvc_release,
    .unlocked_ioctl = ksvc_ioctl,
    .compat_ioctl   = ksvc_ioctl,
    .mmap           = ksvc_mmap,
};

static struct miscdevice ksvc_misc = {
    .minor = MISC_DYNAMIC_MINOR,
    .name  = "ksvc",
    .fops  = &ksvc_fops,
    .mode  = 0666,
};

/* ── Module init/exit ── */

static int __init ksvc_init(void)
{
    int ret = misc_register(&ksvc_misc);
    if (ret) {
        pr_err("ksvc: failed to register misc device: %d\n", ret);
        return ret;
    }
    pr_info("ksvc: loaded v%d (Phase 0: rings + shared page)\n",
            KSVC_VERSION);
    return 0;
}

static void __exit ksvc_exit(void)
{
    misc_deregister(&ksvc_misc);
    pr_info("ksvc: unloaded\n");
}

module_init(ksvc_init);
module_exit(ksvc_exit);
