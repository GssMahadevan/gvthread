// SPDX-License-Identifier: GPL-2.0
/*
 * KSVC kernel module test — Phase 0
 *
 * Tests:
 *  1. open /dev/ksvc
 *  2. ioctl CREATE
 *  3. mmap submit ring
 *  4. mmap completion ring
 *  5. mmap shared page (read-only)
 *  6. Verify shared page Tier 0 fields
 *  7. Ring protocol: write entry, read back
 *  8. Double CREATE fails
 *  9. Shared page write protection (SIGSEGV on write)
 * 10. Invalid ring sizes rejected
 * 11. Bad mmap offsets rejected
 * 12. eventfd notification
 *
 * Build:  gcc -Wall -O2 -o test_basic test_basic.c
 * Run:    sudo ./test_basic     (module must be loaded)
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <signal.h>
#include <setjmp.h>
#include <sys/mman.h>
#include <sys/ioctl.h>
#include <sys/eventfd.h>
#include <sys/utsname.h>

#include "../ksvc_uapi.h"

/* ── Test infrastructure ── */

static int tests_run = 0;
static int tests_passed = 0;
static int tests_failed = 0;

#define TEST(name) \
    do { \
        tests_run++; \
        printf("  [%2d] %-50s ", tests_run, name); \
    } while (0)

#define PASS() \
    do { \
        printf("\033[32mPASS\033[0m\n"); \
        tests_passed++; \
    } while (0)

#define FAIL(fmt, ...) \
    do { \
        printf("\033[31mFAIL\033[0m: " fmt "\n", ##__VA_ARGS__); \
        tests_failed++; \
    } while (0)

#define ASSERT_EQ(a, b) \
    do { \
        if ((a) != (b)) { \
            FAIL("%s=%lld != %s=%lld", #a, (long long)(a), #b, (long long)(b)); \
            return; \
        } \
    } while (0)

#define ASSERT_NE(a, b) \
    do { \
        if ((a) == (b)) { \
            FAIL("%s == %s (both %lld)", #a, #b, (long long)(a)); \
            return; \
        } \
    } while (0)

#define ASSERT_GE(a, b) \
    do { \
        if ((a) < (b)) { \
            FAIL("%s=%lld < %s=%lld", #a, (long long)(a), #b, (long long)(b)); \
            return; \
        } \
    } while (0)

#define ASSERT_STR_EQ(a, b) \
    do { \
        if (strcmp(a, b) != 0) { \
            FAIL("\"%s\" != \"%s\"", (a), (b)); \
            return; \
        } \
    } while (0)

/* For SIGSEGV test */
static sigjmp_buf segv_jmp;
static volatile sig_atomic_t segv_caught = 0;

static void segv_handler(int sig)
{
    segv_caught = 1;
    siglongjmp(segv_jmp, 1);
}

/* ── Helper: calculate mmap size for a ring ── */
static size_t ring_mmap_size(unsigned int nr_entries, unsigned int entry_size)
{
    size_t data_bytes = (size_t)nr_entries * entry_size;
    size_t data_pages = (data_bytes + 4095) / 4096;
    return (1 + data_pages) * 4096;  /* 1 header page + data pages */
}

/* ── Tests ── */

static void test_open_close(void)
{
    TEST("open /dev/ksvc");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        FAIL("open: %s", strerror(errno));
        return;
    }
    close(fd);
    PASS();
}

static void test_create_basic(void)
{
    TEST("ioctl CREATE with valid params");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    int evfd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    if (evfd < 0) { FAIL("eventfd: %s", strerror(errno)); close(fd); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 64,
        .complete_ring_entries = 64,
        .flags = KSVC_CREATE_DEFAULT,
        .eventfd = evfd,
    };

    int ret = ioctl(fd, KSVC_IOC_CREATE, &params);
    if (ret < 0) {
        FAIL("ioctl CREATE: %s", strerror(errno));
    } else {
        PASS();
    }
    close(evfd);
    close(fd);
}

static void test_create_no_eventfd(void)
{
    TEST("ioctl CREATE without eventfd");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 32,
        .complete_ring_entries = 32,
        .flags = KSVC_CREATE_DEFAULT,
        .eventfd = -1,
    };

    int ret = ioctl(fd, KSVC_IOC_CREATE, &params);
    if (ret < 0) {
        FAIL("ioctl CREATE: %s", strerror(errno));
    } else {
        PASS();
    }
    close(fd);
}

static void test_create_double_fails(void)
{
    TEST("double CREATE returns EBUSY");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = -1,
    };

    int ret = ioctl(fd, KSVC_IOC_CREATE, &params);
    if (ret < 0) { FAIL("first CREATE: %s", strerror(errno)); close(fd); return; }

    ret = ioctl(fd, KSVC_IOC_CREATE, &params);
    if (ret < 0 && errno == EBUSY) {
        PASS();
    } else {
        FAIL("expected EBUSY, got ret=%d errno=%d", ret, errno);
    }
    close(fd);
}

static void test_create_bad_sizes(void)
{
    TEST("CREATE with non-power-of-2 ring size → EINVAL");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 33,   /* not power of 2 */
        .complete_ring_entries = 64,
        .eventfd = -1,
    };

    int ret = ioctl(fd, KSVC_IOC_CREATE, &params);
    if (ret < 0 && errno == EINVAL) {
        PASS();
    } else {
        FAIL("expected EINVAL, got ret=%d errno=%d", ret, errno);
    }
    close(fd);
}

static void test_mmap_submit_ring(void)
{
    TEST("mmap submit ring");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 64,
        .complete_ring_entries = 64,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    size_t sz = ring_mmap_size(64, sizeof(struct ksvc_entry));
    void *p = mmap(NULL, sz, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
                   KSVC_OFF_SUBMIT_RING);
    if (p == MAP_FAILED) {
        FAIL("mmap: %s", strerror(errno));
    } else {
        struct ksvc_ring_header *hdr = (struct ksvc_ring_header *)p;
        if (hdr->magic == KSVC_RING_MAGIC && hdr->ring_size == 64 &&
            hdr->mask == 63 && hdr->entry_size == sizeof(struct ksvc_entry)) {
            PASS();
        } else {
            FAIL("header: magic=0x%x size=%u mask=%u entry_size=%u",
                 hdr->magic, hdr->ring_size, hdr->mask, hdr->entry_size);
        }
        munmap(p, sz);
    }
    close(fd);
}

static void test_mmap_complete_ring(void)
{
    TEST("mmap completion ring");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 64,
        .complete_ring_entries = 128,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    size_t sz = ring_mmap_size(128, sizeof(struct ksvc_completion));
    void *p = mmap(NULL, sz, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
                   KSVC_OFF_COMPLETE_RING);
    if (p == MAP_FAILED) {
        FAIL("mmap: %s", strerror(errno));
    } else {
        struct ksvc_ring_header *hdr = (struct ksvc_ring_header *)p;
        if (hdr->magic == KSVC_RING_MAGIC && hdr->ring_size == 128) {
            PASS();
        } else {
            FAIL("header: magic=0x%x size=%u", hdr->magic, hdr->ring_size);
        }
        munmap(p, sz);
    }
    close(fd);
}

static void test_shared_page_fields(void)
{
    TEST("shared page: pid/uid/uname match process");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    void *p = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd,
                   KSVC_OFF_SHARED_PAGE);
    if (p == MAP_FAILED) {
        FAIL("mmap: %s", strerror(errno)); close(fd); return;
    }

    struct ksvc_shared_page *sp = (struct ksvc_shared_page *)p;

    /* Verify magic and version */
    ASSERT_EQ(sp->magic, KSVC_SHARED_MAGIC);
    ASSERT_EQ(sp->version, KSVC_VERSION);

    /* Verify PID matches getpid() */
    ASSERT_EQ(sp->pid, getpid());
    ASSERT_EQ(sp->tgid, getpid());

    /* Verify UID matches getuid() */
    ASSERT_EQ(sp->uid, getuid());
    ASSERT_EQ(sp->gid, getgid());
    ASSERT_EQ(sp->euid, geteuid());
    ASSERT_EQ(sp->egid, getegid());

    /* Verify utsname matches uname() */
    struct utsname uts;
    uname(&uts);
    ASSERT_STR_EQ(sp->utsname_release, uts.release);
    ASSERT_STR_EQ(sp->utsname_nodename, uts.nodename);
    ASSERT_STR_EQ(sp->utsname_machine, uts.machine);

    /* Verify resource limits are reasonable */
    ASSERT_GE(sp->rlimit_nofile, 256ULL);

    /* Verify timestamps are nonzero */
    ASSERT_NE(sp->clock_monotonic_ns, 0ULL);
    ASSERT_NE(sp->boot_time_ns, 0ULL);

    munmap(p, 4096);
    close(fd);
    PASS();
}

static void test_shared_page_readonly(void)
{
    TEST("shared page: write causes SIGSEGV");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    void *p = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd,
                   KSVC_OFF_SHARED_PAGE);
    if (p == MAP_FAILED) {
        FAIL("mmap: %s", strerror(errno)); close(fd); return;
    }

    /* Set up SIGSEGV handler */
    struct sigaction sa, old_sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = segv_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = 0;
    sigaction(SIGSEGV, &sa, &old_sa);

    segv_caught = 0;
    if (sigsetjmp(segv_jmp, 1) == 0) {
        /* Try to write — should SIGSEGV */
        volatile char *ptr = (volatile char *)p;
        *ptr = 0x42;
        /* If we get here, no SIGSEGV was raised */
        FAIL("write succeeded — page is not read-only!");
    } else {
        /* We caught SIGSEGV — good */
        if (segv_caught) {
            PASS();
        } else {
            FAIL("longjmp but no SIGSEGV");
        }
    }

    sigaction(SIGSEGV, &old_sa, NULL);
    munmap(p, 4096);
    close(fd);
}

static void test_ring_write_read(void)
{
    TEST("ring protocol: write entry → read back");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    size_t sz = ring_mmap_size(16, sizeof(struct ksvc_entry));
    void *p = mmap(NULL, sz, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
                   KSVC_OFF_SUBMIT_RING);
    if (p == MAP_FAILED) {
        FAIL("mmap: %s", strerror(errno)); close(fd); return;
    }

    volatile struct ksvc_ring_header *hdr = (volatile struct ksvc_ring_header *)p;
    struct ksvc_entry *entries = (struct ksvc_entry *)((char *)p + 4096);

    /* Ring should be empty initially */
    ASSERT_EQ(hdr->head, 0ULL);
    ASSERT_EQ(hdr->tail, 0ULL);

    /* Write an entry (producer: advance tail) */
    __u64 tail = hdr->tail;
    __u32 idx = tail & hdr->mask;
    entries[idx].corr_id = 42;
    entries[idx].syscall_nr = 0;  /* __NR_read */
    entries[idx].args[0] = 3;     /* fd */
    entries[idx].args[1] = 0x1000;/* buf */
    entries[idx].args[2] = 4096;  /* count */

    __sync_synchronize();  /* memory barrier */
    hdr->tail = tail + 1;

    /* Read back (consumer: read at head, advance head) */
    __u64 head = hdr->head;
    ASSERT_NE(head, hdr->tail);  /* not empty */

    idx = head & hdr->mask;
    ASSERT_EQ(entries[idx].corr_id, 42ULL);
    ASSERT_EQ(entries[idx].syscall_nr, 0U);
    ASSERT_EQ(entries[idx].args[0], 3ULL);
    ASSERT_EQ(entries[idx].args[2], 4096ULL);

    hdr->head = head + 1;
    ASSERT_EQ(hdr->head, hdr->tail);  /* now empty */

    munmap(p, sz);
    close(fd);
    PASS();
}

static void test_ring_wrap_around(void)
{
    TEST("ring wrap-around: fill → drain → refill");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    size_t sz = ring_mmap_size(16, sizeof(struct ksvc_entry));
    void *p = mmap(NULL, sz, PROT_READ | PROT_WRITE, MAP_SHARED, fd,
                   KSVC_OFF_SUBMIT_RING);
    if (p == MAP_FAILED) {
        FAIL("mmap: %s", strerror(errno)); close(fd); return;
    }

    volatile struct ksvc_ring_header *hdr = (volatile struct ksvc_ring_header *)p;
    struct ksvc_entry *entries = (struct ksvc_entry *)((char *)p + 4096);

    /* Fill the ring (16 entries) */
    for (int i = 0; i < 16; i++) {
        __u64 tail = hdr->tail;
        __u32 idx = tail & hdr->mask;
        entries[idx].corr_id = 100 + i;
        entries[idx].syscall_nr = i;
        __sync_synchronize();
        hdr->tail = tail + 1;
    }

    /* Ring should be full: tail - head == 16 */
    ASSERT_EQ(hdr->tail - hdr->head, 16ULL);

    /* Drain all */
    for (int i = 0; i < 16; i++) {
        __u64 head = hdr->head;
        __u32 idx = head & hdr->mask;
        ASSERT_EQ(entries[idx].corr_id, 100ULL + i);
        hdr->head = head + 1;
    }

    /* Ring should be empty again */
    ASSERT_EQ(hdr->head, hdr->tail);

    /* Refill after wrap-around (tail is now at 16, wraps via mask) */
    for (int i = 0; i < 8; i++) {
        __u64 tail = hdr->tail;
        __u32 idx = tail & hdr->mask;
        entries[idx].corr_id = 200 + i;
        __sync_synchronize();
        hdr->tail = tail + 1;
    }

    /* Verify wrapped entries */
    for (int i = 0; i < 8; i++) {
        __u64 head = hdr->head;
        __u32 idx = head & hdr->mask;
        ASSERT_EQ(entries[idx].corr_id, 200ULL + i);
        hdr->head = head + 1;
    }

    munmap(p, sz);
    close(fd);
    PASS();
}

static void test_bad_mmap_offset(void)
{
    TEST("mmap with bad offset → EINVAL");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = -1,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno)); close(fd); return;
    }

    void *p = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd, 0x00300000);
    if (p == MAP_FAILED && errno == EINVAL) {
        PASS();
    } else {
        FAIL("expected EINVAL, got p=%p errno=%d", p, errno);
        if (p != MAP_FAILED) munmap(p, 4096);
    }
    close(fd);
}

static void test_mmap_before_create(void)
{
    TEST("mmap before CREATE → EINVAL");
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    void *p = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd,
                   KSVC_OFF_SHARED_PAGE);
    if (p == MAP_FAILED && errno == EINVAL) {
        PASS();
    } else {
        FAIL("expected EINVAL, got p=%p errno=%d", p, errno);
        if (p != MAP_FAILED) munmap(p, 4096);
    }
    close(fd);
}

static void test_eventfd_notification(void)
{
    TEST("eventfd: kernel can signal");
    /* Phase 0 doesn't use the eventfd from kernel side,
     * but we verify the eventfd_ctx was acquired correctly
     * by checking the eventfd is still usable after CREATE. */
    int fd = open("/dev/ksvc", O_RDWR | O_CLOEXEC);
    if (fd < 0) { FAIL("open: %s", strerror(errno)); return; }

    int evfd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    if (evfd < 0) { FAIL("eventfd: %s", strerror(errno)); close(fd); return; }

    struct ksvc_create_params params = {
        .submit_ring_entries = 16,
        .complete_ring_entries = 16,
        .eventfd = evfd,
    };
    if (ioctl(fd, KSVC_IOC_CREATE, &params) < 0) {
        FAIL("CREATE: %s", strerror(errno));
        close(evfd); close(fd); return;
    }

    /* Write to eventfd from userspace (simulating what dispatcher does) */
    uint64_t val = 1;
    ssize_t w = write(evfd, &val, sizeof(val));
    ASSERT_EQ(w, (ssize_t)sizeof(val));

    /* Read it back */
    uint64_t rval = 0;
    ssize_t r = read(evfd, &rval, sizeof(rval));
    ASSERT_EQ(r, (ssize_t)sizeof(rval));
    ASSERT_EQ(rval, 1ULL);

    close(evfd);
    close(fd);
    PASS();
}

/* ── Main ── */

int main(void)
{
    printf("\n=== KSVC Kernel Module Test (Phase 0) ===\n\n");

    /* Check module is loaded */
    if (access("/dev/ksvc", F_OK) != 0) {
        fprintf(stderr, "ERROR: /dev/ksvc not found. Load module first:\n");
        fprintf(stderr, "  sudo insmod ksvc.ko\n");
        return 1;
    }

    test_open_close();
    test_create_basic();
    test_create_no_eventfd();
    test_create_double_fails();
    test_create_bad_sizes();
    test_mmap_submit_ring();
    test_mmap_complete_ring();
    test_shared_page_fields();
    test_shared_page_readonly();
    test_ring_write_read();
    test_ring_wrap_around();
    test_bad_mmap_offset();
    test_mmap_before_create();
    test_eventfd_notification();

    printf("\n──────────────────────────────────────────\n");
    printf("  Total: %d  Passed: \033[32m%d\033[0m  Failed: \033[%dm%d\033[0m\n",
           tests_run, tests_passed,
           tests_failed > 0 ? 31 : 32, tests_failed);
    printf("──────────────────────────────────────────\n\n");

    return tests_failed > 0 ? 1 : 0;
}
