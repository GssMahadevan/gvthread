//! KSVC End-to-End Smoke Test
//!
//! Tests the full KSVC stack:
//!   Part A — Kernel Module (/dev/ksvc): shared page, rings, eventfd
//!   Part B — Routing table: tier assignment for known syscalls
//!   Part C — Tier 1 (io_uring): openat, read, write, statx, close, network
//!   Part D — Tier 2 (worker pool): dup, lseek via libc::syscall()
//!
//! Run: sudo ./target/release/ksvc-smoke
//! (sudo needed for /dev/ksvc; Parts B-D work without sudo)

use ksvc_core::entry::{CorrId, SubmitEntry};
use ksvc_core::io_backend::{IoBackend, IoCompletion};
use ksvc_core::router::SyscallRouter;
use ksvc_core::shared_page::SharedPage;
use ksvc_core::tier::Tier;
use ksvc_core::worker::{WorkerCompletion, WorkerPool};

use ksvc_module::instance::InstanceBuilder;
use ksvc_module::ksvc_sys;
use ksvc_module::mmap_shared_page::MmapSharedPage;

use std::ffi::CString;
use std::io::Write;

// ── Linux x86_64 syscall numbers ──
mod nr {
    pub const READ: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const CLOSE: u32 = 3;
    pub const LSEEK: u32 = 8;
    pub const DUP: u32 = 32;
    pub const GETPID: u32 = 39;
    pub const SENDTO: u32 = 44;
    pub const RECVFROM: u32 = 45;
    pub const SETSOCKOPT: u32 = 54;
    pub const GETUID: u32 = 102;
    pub const GETGID: u32 = 104;
    pub const GETPPID: u32 = 110;
    pub const OPENAT: u32 = 257;
    pub const ACCEPT4: u32 = 288;
    pub const STATX: u32 = 332;
}

// ── Test harness ──

struct TestRunner {
    total: usize,
    passed: usize,
    failed: usize,
}

const LINE: &str = "────────────────────────────────────────────────────────────";

impl TestRunner {
    fn new() -> Self {
        Self { total: 0, passed: 0, failed: 0 }
    }

    fn section(&self, name: &str) {
        println!("\n{}", LINE);
        println!("  {}", name);
        println!("{}", LINE);
    }

    fn pass(&mut self, name: &str) {
        self.total += 1;
        self.passed += 1;
        println!("  [{:2}] {:<52} PASS", self.total, name);
    }

    fn fail(&mut self, name: &str, reason: &str) {
        self.total += 1;
        self.failed += 1;
        println!("  [{:2}] {:<52} FAIL: {}", self.total, name, reason);
    }

    fn check(&mut self, name: &str, ok: bool, reason: &str) {
        if ok { self.pass(name); } else { self.fail(name, reason); }
    }

    fn summary(&self) {
        println!("\n{}", LINE);
        println!(
            "  Total: {}  Passed: {}  Failed: {}",
            self.total, self.passed, self.failed
        );
        println!("{}", LINE);
    }
}

/// Helper: submit one SQE, flush, poll until completion (with timeout).
fn submit_and_wait(
    io: &mut ksvc_module::basic_iouring::BasicIoUring,
    entry: &SubmitEntry,
    opcode: u8,
) -> Option<IoCompletion> {
    if io.submit_with_opcode(entry, opcode).is_err() {
        return None;
    }
    if io.flush().is_err() {
        return None;
    }
    let mut buf = [IoCompletion { corr_id: CorrId(0), result: 0, flags: 0 }; 4];
    for _ in 0..200 {
        let n = io.poll_completions(&mut buf, 4);
        if n > 0 {
            return Some(buf[0]);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    None
}

// ════════════════════════════════════════════════════════════
// Part A: Kernel Module
// ════════════════════════════════════════════════════════════

fn test_kmod(t: &mut TestRunner) -> Option<(*const u8, i32)> {
    t.section("Part A: Kernel Module (/dev/ksvc)");

    // A1: Open /dev/ksvc
    let ksvc_fd = match ksvc_sys::open_ksvc() {
        Ok(fd) => { t.pass("open /dev/ksvc"); fd }
        Err(e) => {
            t.fail("open /dev/ksvc", &format!("{} (is ksvc.ko loaded?)", e));
            println!("       Skipping kernel module tests.");
            return None;
        }
    };

    // A2: eventfd
    let efd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    t.check("eventfd()", efd >= 0, "eventfd failed");

    // A3: ioctl CREATE
    let mut params = ksvc_sys::KsvcCreateParams::default();
    params.eventfd = efd;
    params.submit_ring_entries = 64;
    params.complete_ring_entries = 64;
    let create_ok = unsafe { ksvc_sys::ksvc_ioc_create(ksvc_fd, &mut params) };
    t.check("ioctl CREATE", create_ok.is_ok(), &format!("{:?}", create_ok.err()));

    // A4: mmap shared page
    let shared_ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(), 4096, libc::PROT_READ, libc::MAP_SHARED,
            ksvc_fd, ksvc_sys::KSVC_OFF_SHARED_PAGE as i64,
        )
    };
    t.check("mmap shared page", shared_ptr != libc::MAP_FAILED, "MAP_FAILED");
    if shared_ptr == libc::MAP_FAILED {
        unsafe { libc::close(ksvc_fd); }
        return None;
    }

    // A5: Magic + version
    let sp = unsafe { &*(shared_ptr as *const ksvc_sys::KsvcSharedPageLayout) };
    t.check(
        "shared page magic + version",
        sp.magic == ksvc_sys::KSVC_SHARED_MAGIC && sp.version == ksvc_sys::KSVC_VERSION,
        &format!("magic=0x{:08X} ver={}", sp.magic, sp.version),
    );

    // A6-A8: Tier 0 identity
    let pid = unsafe { libc::getpid() };
    let uid = unsafe { libc::getuid() };
    let ppid = unsafe { libc::getppid() };
    t.check("Tier 0: pid == getpid()", sp.pid == pid, &format!("{} vs {}", sp.pid, pid));
    t.check("Tier 0: uid == getuid()", sp.uid == uid, &format!("{} vs {}", sp.uid, uid));
    t.check("Tier 0: ppid == getppid()", sp.ppid == ppid, &format!("{} vs {}", sp.ppid, ppid));

    // A9: MmapSharedPage Rust wrapper
    let msp = unsafe { MmapSharedPage::new(shared_ptr as *const u8) };
    t.check(
        "MmapSharedPage wrapper matches",
        msp.pid() == pid && msp.uid() == uid && msp.ppid() == ppid,
        "field mismatch",
    );

    // A10: submit ring header
    let submit_ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(), 4096 * 4,
            libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED,
            ksvc_fd, ksvc_sys::KSVC_OFF_SUBMIT_RING as i64,
        )
    };
    if submit_ptr != libc::MAP_FAILED {
        let hdr = unsafe { &*(submit_ptr as *const ksvc_sys::KsvcRingHeader) };
        t.check(
            "submit ring: magic + size=64",
            hdr.magic == ksvc_sys::KSVC_RING_MAGIC && hdr.ring_size == 64,
            &format!("magic=0x{:08X} size={}", hdr.magic, hdr.ring_size),
        );
        unsafe { libc::munmap(submit_ptr, 4096 * 4); }
    } else {
        t.fail("mmap submit ring", "MAP_FAILED");
    }

    Some((shared_ptr as *const u8, ksvc_fd))
}

// ════════════════════════════════════════════════════════════
// Part B: Routing Table
// ════════════════════════════════════════════════════════════

fn test_routing(t: &mut TestRunner, inst: &ksvc_module::instance::DefaultInstance) {
    t.section("Part B: Routing Table");

    // Tier 0
    let t0 = matches!(inst.router.route(nr::GETPID).tier, Tier::SharedPage)
        && matches!(inst.router.route(nr::GETUID).tier, Tier::SharedPage)
        && matches!(inst.router.route(nr::GETGID).tier, Tier::SharedPage)
        && matches!(inst.router.route(nr::GETPPID).tier, Tier::SharedPage);
    t.check("Tier 0: getpid/getuid/getgid/getppid", t0, "tier mismatch");

    // Tier 1
    let t1 = matches!(inst.router.route(nr::READ).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::WRITE).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::OPENAT).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::CLOSE).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::STATX).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::ACCEPT4).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::SENDTO).tier, Tier::IoUring)
        && matches!(inst.router.route(nr::RECVFROM).tier, Tier::IoUring);
    t.check("Tier 1: read/write/openat/close/statx/accept/send/recv", t1, "tier mismatch");

    // Tier 2
    let t2 = matches!(inst.router.route(nr::SETSOCKOPT).tier, Tier::WorkerPool)
        && matches!(inst.router.route(nr::DUP).tier, Tier::WorkerPool)
        && matches!(inst.router.route(nr::LSEEK).tier, Tier::WorkerPool);
    t.check("Tier 2: setsockopt/dup/lseek", t2, "tier mismatch");

    // Counts
    let c = inst.router.tier_counts();
    t.check(
        &format!("tier counts: T0={} T1={} T2={} T3={}", c.tier0, c.tier1, c.tier2, c.tier3),
        c.tier0 >= 5 && c.tier1 >= 15,
        "low tier counts",
    );
}

// ════════════════════════════════════════════════════════════
// Part C: Tier 1 — io_uring
// ════════════════════════════════════════════════════════════

fn test_tier1(t: &mut TestRunner, inst: &mut ksvc_module::instance::DefaultInstance) {
    t.section("Part C: Tier 1 — io_uring Syscall Execution");

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("ksvc_smoke_test.txt");
    let test_data = b"hello from ksvc smoke test!\n";
    std::fs::File::create(&tmp_path)
        .and_then(|mut f| f.write_all(test_data))
        .expect("create temp file");
    let c_path = CString::new(tmp_path.to_str().unwrap()).unwrap();

    // C1: openat
    let route_open = inst.router.route(nr::OPENAT);
    let entry = SubmitEntry {
        corr_id: CorrId(1), syscall_nr: nr::OPENAT, flags: 0,
        args: [libc::AT_FDCWD as u64, c_path.as_ptr() as u64, libc::O_RDONLY as u64, 0, 0, 0],
    };
    let comp = submit_and_wait(&mut inst.io_backend, &entry, route_open.iouring_opcode);
    let open_fd = comp.map(|c| c.result).unwrap_or(-999);
    t.check(&format!("openat(O_RDONLY) -> fd={}", open_fd), open_fd >= 0, &format!("err={}", -open_fd));
    if open_fd < 0 { let _ = std::fs::remove_file(&tmp_path); return; }

    // C2: read
    let mut rbuf = [0u8; 128];
    let route_r = inst.router.route(nr::READ);
    let entry_r = SubmitEntry {
        corr_id: CorrId(2), syscall_nr: nr::READ, flags: 0,
        args: [open_fd as u64, rbuf.as_mut_ptr() as u64, rbuf.len() as u64, 0, 0, 0],
    };
    let read_n = submit_and_wait(&mut inst.io_backend, &entry_r, route_r.iouring_opcode)
        .map(|c| c.result).unwrap_or(-1);
    t.check(&format!("read -> {} bytes", read_n), read_n == test_data.len() as i64,
        &format!("expected {}", test_data.len()));

    // C3: content
    if read_n > 0 {
        t.check("read content matches", &rbuf[..test_data.len()] == test_data, "mismatch");
    }

    // C4: statx
    let route_sx = inst.router.route(nr::STATX);
    let mut sxbuf: io_uring::types::statx = unsafe { std::mem::zeroed() };
    let entry_sx = SubmitEntry {
        corr_id: CorrId(3), syscall_nr: nr::STATX, flags: 0,
        args: [libc::AT_FDCWD as u64, c_path.as_ptr() as u64, 0, libc::STATX_SIZE as u64,
               &mut sxbuf as *mut _ as u64, 0],
    };
    let sx_ret = submit_and_wait(&mut inst.io_backend, &entry_sx, route_sx.iouring_opcode)
        .map(|c| c.result).unwrap_or(-1);
    t.check("statx completed", sx_ret == 0, &format!("ret={}", sx_ret));
    if sx_ret == 0 {
        t.check(&format!("statx size={}", sxbuf.stx_size),
            sxbuf.stx_size == test_data.len() as u64, &format!("expected {}", test_data.len()));
    }

    // C5: close
    let route_cl = inst.router.route(nr::CLOSE);
    let cl_ret = submit_and_wait(&mut inst.io_backend,
        &SubmitEntry { corr_id: CorrId(4), syscall_nr: nr::CLOSE, flags: 0,
            args: [open_fd as u64, 0, 0, 0, 0, 0] },
        route_cl.iouring_opcode).map(|c| c.result).unwrap_or(-1);
    t.check("close completed", cl_ret == 0, &format!("ret={}", cl_ret));

    // C6-C7: write cycle (create, write, close, readback)
    let wpath = tmp_dir.join("ksvc_smoke_write.txt");
    let c_wpath = CString::new(wpath.to_str().unwrap()).unwrap();
    let wdata = b"written by ksvc io_uring!\n";

    let wfd = submit_and_wait(&mut inst.io_backend,
        &SubmitEntry { corr_id: CorrId(5), syscall_nr: nr::OPENAT, flags: 0,
            args: [libc::AT_FDCWD as u64, c_wpath.as_ptr() as u64,
                   (libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC) as u64, 0o644u64, 0, 0] },
        route_open.iouring_opcode).map(|c| c.result).unwrap_or(-1);
    t.check(&format!("openat(O_CREAT) -> fd={}", wfd), wfd >= 0, &format!("err={}", -wfd));

    if wfd >= 0 {
        let route_w = inst.router.route(nr::WRITE);
        let wn = submit_and_wait(&mut inst.io_backend,
            &SubmitEntry { corr_id: CorrId(6), syscall_nr: nr::WRITE, flags: 0,
                args: [wfd as u64, wdata.as_ptr() as u64, wdata.len() as u64, 0, 0, 0] },
            route_w.iouring_opcode).map(|c| c.result).unwrap_or(-1);
        t.check(&format!("write -> {} bytes", wn), wn == wdata.len() as i64,
            &format!("expected {}", wdata.len()));

        // close
        let _ = submit_and_wait(&mut inst.io_backend,
            &SubmitEntry { corr_id: CorrId(7), syscall_nr: nr::CLOSE, flags: 0,
                args: [wfd as u64, 0, 0, 0, 0, 0] },
            route_cl.iouring_opcode);

        // readback
        let rb = std::fs::read(&wpath).unwrap_or_default();
        t.check("write content verified via readback", rb == wdata, "mismatch");
        let _ = std::fs::remove_file(&wpath);
    }

    // C8-C11: Network roundtrip
    test_tier1_network(t, inst);

    let _ = std::fs::remove_file(&tmp_path);
}

fn test_tier1_network(t: &mut TestRunner, inst: &mut ksvc_module::instance::DefaultInstance) {
    // Setup listener via normal syscalls
    let listener = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if listener < 0 { t.fail("net: socket()", "failed"); return; }

    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as u16;
    addr.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
    addr.sin_port = 0;
    if unsafe { libc::bind(listener, &addr as *const _ as *const _, std::mem::size_of_val(&addr) as u32) } < 0 {
        t.fail("net: bind()", "failed");
        unsafe { libc::close(listener); }
        return;
    }
    unsafe { libc::listen(listener, 1); }

    let mut bound: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut alen: libc::socklen_t = std::mem::size_of::<libc::sockaddr_in>() as u32;
    unsafe { libc::getsockname(listener, &mut bound as *mut _ as *mut _, &mut alen); }

    let client = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as u16;
    sa.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
    sa.sin_port = bound.sin_port;
    unsafe { libc::connect(client, &sa as *const _ as *const _, std::mem::size_of_val(&sa) as u32); }

    // accept4 via io_uring
    let route_acc = inst.router.route(nr::ACCEPT4);
    let mut peer: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut plen: libc::socklen_t = std::mem::size_of::<libc::sockaddr_in>() as u32;
    let acc_fd = submit_and_wait(&mut inst.io_backend,
        &SubmitEntry { corr_id: CorrId(10), syscall_nr: nr::ACCEPT4, flags: 0,
            args: [listener as u64, &mut peer as *mut _ as u64,
                   &mut plen as *mut _ as u64, libc::SOCK_CLOEXEC as u64, 0, 0] },
        route_acc.iouring_opcode).map(|c| c.result).unwrap_or(-1);
    t.check(&format!("accept4 -> fd={}", acc_fd), acc_fd >= 0, &format!("err={}", -acc_fd));

    if acc_fd < 0 {
        unsafe { libc::close(listener); libc::close(client); }
        return;
    }

    // Client sends, server recvs via io_uring
    let msg = b"ping from ksvc!";
    unsafe { libc::send(client, msg.as_ptr() as *const _, msg.len(), 0); }

    let route_recv = inst.router.route(nr::RECVFROM);
    let mut rbuf = [0u8; 64];
    let recv_n = submit_and_wait(&mut inst.io_backend,
        &SubmitEntry { corr_id: CorrId(11), syscall_nr: nr::RECVFROM, flags: 0,
            args: [acc_fd as u64, rbuf.as_mut_ptr() as u64, rbuf.len() as u64, 0, 0, 0] },
        route_recv.iouring_opcode).map(|c| c.result).unwrap_or(-1);
    t.check(&format!("recv -> {} bytes", recv_n), recv_n == msg.len() as i64,
        &format!("expected {}", msg.len()));
    if recv_n > 0 {
        t.check("recv content matches", &rbuf[..msg.len()] == msg, "mismatch");
    }

    // Server sends reply via io_uring, client verifies
    let reply = b"pong from ksvc!";
    let route_send = inst.router.route(nr::SENDTO);
    let send_n = submit_and_wait(&mut inst.io_backend,
        &SubmitEntry { corr_id: CorrId(12), syscall_nr: nr::SENDTO, flags: 0,
            args: [acc_fd as u64, reply.as_ptr() as u64, reply.len() as u64, 0, 0, 0] },
        route_send.iouring_opcode).map(|c| c.result).unwrap_or(-1);
    t.check(&format!("send -> {} bytes", send_n), send_n == reply.len() as i64,
        &format!("expected {}", reply.len()));

    if send_n > 0 {
        let mut cbuf = [0u8; 64];
        let n = unsafe { libc::recv(client, cbuf.as_mut_ptr() as *mut _, 64, 0) };
        t.check("roundtrip: client got reply",
            n == reply.len() as isize && &cbuf[..reply.len()] == reply, "mismatch");
    }

    unsafe {
        libc::close(acc_fd as i32);
        libc::close(client);
        libc::close(listener);
    }
}

// ════════════════════════════════════════════════════════════
// Part D: Tier 2 — Worker Pool
// ════════════════════════════════════════════════════════════

fn test_tier2(t: &mut TestRunner, inst: &mut ksvc_module::instance::DefaultInstance) {
    t.section("Part D: Tier 2 — Worker Pool Execution");

    // D1: dup(fd) via worker pool
    let tmp = std::env::temp_dir().join("ksvc_smoke_dup.txt");
    let f = std::fs::File::create(&tmp).expect("create dup file");
    let orig_fd = { use std::os::unix::io::AsRawFd; f.as_raw_fd() };

    let route = inst.router.route(nr::DUP);
    t.check("route: dup -> Tier 2", matches!(route.tier, Tier::WorkerPool),
        &format!("{:?}", route.tier));

    let entry = SubmitEntry {
        corr_id: CorrId(20), syscall_nr: nr::DUP, flags: 0,
        args: [orig_fd as u64, 0, 0, 0, 0, 0],
    };
    let _ = inst.worker_pool.enqueue(&entry);

    let mut wbuf = [WorkerCompletion { corr_id: CorrId(0), result: 0 }; 4];
    let mut dup_fd: i64 = i64::MIN;
    for _ in 0..200 {
        if inst.worker_pool.poll_completions(&mut wbuf, 4) > 0 {
            dup_fd = wbuf[0].result;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    t.check(&format!("dup -> fd={}", dup_fd), dup_fd >= 0, &format!("result={}", dup_fd));

    if dup_fd >= 0 {
        let msg = b"dup works!";
        let n = unsafe { libc::write(dup_fd as i32, msg.as_ptr() as *const _, msg.len()) };
        t.check("dup'd fd is writable", n == msg.len() as isize, &format!("wrote {}", n));
        unsafe { libc::close(dup_fd as i32); }
    }
    drop(f);
    let _ = std::fs::remove_file(&tmp);

    // D2: lseek via worker pool
    let tmp2 = std::env::temp_dir().join("ksvc_smoke_lseek.txt");
    { let mut f = std::fs::File::create(&tmp2).unwrap(); f.write_all(b"0123456789").unwrap(); }
    let f2 = std::fs::File::open(&tmp2).unwrap();
    let fd2 = { use std::os::unix::io::AsRawFd; f2.as_raw_fd() };

    t.check("route: lseek -> Tier 2",
        matches!(inst.router.route(nr::LSEEK).tier, Tier::WorkerPool),
        &format!("{:?}", inst.router.route(nr::LSEEK).tier));

    let _ = inst.worker_pool.enqueue(&SubmitEntry {
        corr_id: CorrId(21), syscall_nr: nr::LSEEK, flags: 0,
        args: [fd2 as u64, 5, libc::SEEK_SET as u64, 0, 0, 0],
    });

    let mut lseek_ret: i64 = i64::MIN;
    for _ in 0..200 {
        if inst.worker_pool.poll_completions(&mut wbuf, 4) > 0 {
            lseek_ret = wbuf[0].result;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    t.check(&format!("lseek(5, SEEK_SET) -> {}", lseek_ret), lseek_ret == 5,
        &format!("expected 5 got {}", lseek_ret));

    drop(f2);
    let _ = std::fs::remove_file(&tmp2);
}

// ════════════════════════════════════════════════════════════

fn main() {
    println!("=== KSVC End-to-End Smoke Test ===");
    let kver = std::fs::read_to_string("/proc/version").unwrap_or_default();
    println!("    kernel: {}", kver.trim().split(' ').nth(2).unwrap_or("?"));

    let mut t = TestRunner::new();

    // Part A
    let kmod = test_kmod(&mut t);

    // Build instance for Parts B-D
    let mut inst = match InstanceBuilder::new().sq_entries(64).worker_count(2).build() {
        Ok(i) => i,
        Err(e) => {
            println!("\nFATAL: InstanceBuilder failed: {:?}", e);
            if let Some((p, fd)) = kmod { unsafe { libc::munmap(p as *mut _, 4096); libc::close(fd); } }
            t.summary();
            std::process::exit(1);
        }
    };

    test_routing(&mut t, &inst);
    test_tier1(&mut t, &mut inst);
    test_tier2(&mut t, &mut inst);

    drop(inst);
    if let Some((p, fd)) = kmod {
        unsafe { libc::munmap(p as *mut _, 4096); libc::close(fd); }
    }

    t.summary();
    std::process::exit(if t.failed > 0 { 1 } else { 0 });
}
