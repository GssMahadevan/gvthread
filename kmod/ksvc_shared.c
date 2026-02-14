// SPDX-License-Identifier: GPL-2.0
/*
 * KSVC - Shared page (Tier 0)
 *
 * A single page allocated by the kernel, populated with process metadata,
 * and mmap'd read-only into userspace.
 *
 * Userspace reads these fields with a simple memory load (~4 cycles)
 * instead of a syscall (~200 cycles). This is the Tier 0 optimization.
 *
 * Fields are populated once at CREATE time (identity, credentials, system
 * info) and never change. Runtime stats fields at offset 0x200+ can be
 * updated by the userspace dispatcher via its own write mapping, or by
 * a future kthread.
 */

#include <linux/slab.h>
#include <linux/mm.h>
#include <linux/utsname.h>
#include <linux/pid_namespace.h>
#include <linux/sched/signal.h>
#include <linux/cred.h>
#include <linux/string.h>
#include <linux/ktime.h>

#include "ksvc_internal.h"

int ksvc_shared_alloc(struct ksvc_instance *inst)
{
    inst->shared_page = alloc_page(GFP_KERNEL | __GFP_ZERO);
    if (!inst->shared_page)
        return -ENOMEM;

    inst->shared_kaddr = page_address(inst->shared_page);
    if (!inst->shared_kaddr) {
        __free_page(inst->shared_page);
        inst->shared_page = NULL;
        return -ENOMEM;
    }

    ksvc_dbg("shared_alloc: page at %p\n", inst->shared_kaddr);
    return 0;
}

void ksvc_shared_free(struct ksvc_instance *inst)
{
    if (inst->shared_page) {
        __free_page(inst->shared_page);
        inst->shared_page = NULL;
        inst->shared_kaddr = NULL;
    }
}

/*
 * Populate the shared page with current process context.
 *
 * Called once during CREATE ioctl, in the context of the creating process.
 * All fields are fixed after this — identity and credentials don't change.
 */
void ksvc_shared_populate(struct ksvc_instance *inst)
{
    struct ksvc_shared_page *sp = inst->shared_kaddr;
    const struct cred *cred = current_cred();
    struct new_utsname *uts;
    struct task_struct *task = current;

    if (!sp)
        return;

    /* Magic and version */
    sp->magic = KSVC_SHARED_MAGIC;
    sp->version = KSVC_VERSION;

    /* Process identity */
    sp->pid = task_pid_nr(task);
    sp->tgid = task_tgid_nr(task);
    sp->ppid = task_ppid_nr(task);

    rcu_read_lock();
    sp->pgid = task_pgrp_nr_ns(task, task_active_pid_ns(task));
    sp->sid = task_session_nr_ns(task, task_active_pid_ns(task));
    rcu_read_unlock();

    /* Credentials */
    sp->uid  = from_kuid_munged(current_user_ns(), cred->uid);
    sp->gid  = from_kgid_munged(current_user_ns(), cred->gid);
    sp->euid = from_kuid_munged(current_user_ns(), cred->euid);
    sp->egid = from_kgid_munged(current_user_ns(), cred->egid);
    sp->suid = from_kuid_munged(current_user_ns(), cred->suid);
    sp->sgid = from_kgid_munged(current_user_ns(), cred->sgid);

    /* System info from utsname */
    uts = utsname();
    if (uts) {
        strscpy(sp->utsname_release, uts->release,
                sizeof(sp->utsname_release));
        strscpy(sp->utsname_nodename, uts->nodename,
                sizeof(sp->utsname_nodename));
        strscpy(sp->utsname_machine, uts->machine,
                sizeof(sp->utsname_machine));
    }

    /* Resource limits */
    sp->rlimit_nofile = rlimit(RLIMIT_NOFILE);
    sp->rlimit_nproc = rlimit(RLIMIT_NPROC);

    /* Boot time (fixed) */
    sp->boot_time_ns = ktime_get_boottime_ns();

    /* Initial timestamps */
    sp->clock_monotonic_ns = ktime_get_ns();
    sp->clock_realtime_ns = ktime_get_real_ns();

    ksvc_info("shared page populated: pid=%d uid=%u release=%s\n",
              sp->pid, sp->uid, sp->utsname_release);
}

/*
 * mmap the shared page into userspace.
 *
 * The shared page is mapped READ-ONLY to userspace.
 * The kernel (or future kthread) is the sole writer.
 *
 * For Phase 0, the runtime stats section (0x200+) is not updated
 * by the kernel — the userspace dispatcher can mmap a separate
 * writable copy of its own stats region if needed.
 */
int ksvc_shared_mmap(struct ksvc_instance *inst, struct vm_area_struct *vma)
{
    unsigned long size = vma->vm_end - vma->vm_start;
    int ret;

    if (size != PAGE_SIZE) {
        ksvc_err("shared_mmap: size %lu != PAGE_SIZE\n", size);
        return -EINVAL;
    }

    /* Enforce read-only: clear write permission */
    vm_flags_clear(vma, VM_WRITE | VM_MAYWRITE);
    vma->vm_page_prot = vm_get_page_prot(vma->vm_flags);

    /* Don't copy on fork, don't expand */
    vm_flags_set(vma, VM_DONTCOPY | VM_DONTEXPAND);

    ret = vm_insert_page(vma, vma->vm_start, inst->shared_page);
    if (ret) {
        ksvc_err("shared_mmap: vm_insert_page failed: %d\n", ret);
        return ret;
    }

    ksvc_dbg("shared_mmap: mapped at %lx (read-only)\n", vma->vm_start);
    return 0;
}
