```txt
gdb) bt full
#0  0x00007f8a78e4c4c0 in _int_malloc (av=av@entry=0x7f8a58000030, bytes=bytes@entry=32) at ./malloc/malloc.c:4375
        iters = <optimized out>
        nb = 48
        idx = 4
        bin = <optimized out>
        victim = 0x7f8a58020fd0
        size = 48
        victim_index = <optimized out>
        remainder = <optimized out>
        remainder_size = <optimized out>
        block = <optimized out>
        bit = <optimized out>
        map = 0
        fwd = <optimized out>
        bck = <optimized out>
        tcache_unsorted_count = 0
        tcache_nb = 48
        tc_idx = 1
        return_cached = <optimized out>
        __PRETTY_FUNCTION__ = "_int_malloc"
#1  0x00007f8a78e4d139 in __GI___libc_malloc (bytes=32) at ./malloc/malloc.c:3329
        ar_ptr = 0x7f8a58000030
        victim = <optimized out>
        tbytes = <optimized out>
        tc_idx = <optimized out>
        __PRETTY_FUNCTION__ = "__libc_malloc"
#2  0x000055e1af96623e in alloc::alloc::alloc (layout=...)
    at /home/gssm/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/alloc.rs:95
No locals.
#3  alloc::alloc::Global::alloc_impl (self=0x1, layout=..., zeroed=false)
    at /home/gssm/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/alloc.rs:190
        raw_ptr = <optimized out>
        val = <optimized out>
        size = 32
        ptr = <optimized out>
        residual = <optimized out>
#4  alloc::alloc::{impl#1}::allocate (layout=...)
    at /home/gssm/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/alloc.rs:251
        self = 0x1
#5  alloc::alloc::exchange_malloc (size=32, align=8)
    at /home/gssm/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/alloc.rs:352
        layout = core::alloc::layout::Layout {size: 32, align: core::ptr::alignment::Alignment (core::ptr::alignment::AlignmentEnum::_Align1Shl3)}
        ptr = <optimized out>
#6  alloc::boxed::{impl#0}::new<alloc::sync::ArcInner<gvthread_core::cancel::CancellationInner>> (x=...)
    at /home/gssm/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/boxed.rs:265
No locals.
#7  alloc::sync::Arc<gvthread_core::cancel::CancellationInner, alloc::alloc::Global>::new<gvthread_core::cancel::CancellationInner> (data=...)
    at /home/gssm/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/sync.rs:422
        x = <optimized out>
#8  gvthread_core::cancel::CancellationToken::new () at crates/gvthread-core/src/cancel.rs:35
No locals.
#9  0x000055e1af9635b8 in gvthread_runtime::scheduler::gvthread_entry (closure_ptr=0) at crates/gvthread-runtime/src/scheduler.rs:319
        boxed = 0x55e1d12d52c0
#10 0x000055e1af961b7a in gvthread_runtime::arch::x86_64::gvthread_entry_trampoline ()
No symbol table info available.
#11 0x0000000000000000 in ?? ()
No symbol table info available.

```

The Bug: Heap Allocation from GVThread Stack
What Happened
#0  _int_malloc (av=0x7f8a58000030, ...)   ← malloc arena pointer in GVThread mmap region!
#1  __GI___libc_malloc
#2  alloc::alloc::alloc
...
#8  CancellationToken::new()               ← Arc::new() allocates
#9  gvthread_entry (closure_ptr=0)         ← Running on GVThread's custom stack
Root Cause
glibc malloc uses thread-local storage (TLS) to find the memory arena. When we context-switch to a GVThread:

We switch to GVThread's custom stack (in our mmap'd region at 0x7f8a58...)
RSP (stack pointer) now points into our region
malloc sees the weird stack pointer and gets confused
It either:

Uses wrong arena pointer
Corrupts arena metadata
Accesses unmapped memory → SEGV



Normal thread:          GVThread (broken):
┌─────────────┐         ┌─────────────┐
│ OS Stack    │         │ GVThread    │
│ 0x7fff...   │         │ Stack       │
├─────────────┤         │ 0x7f8a58... │ ← malloc sees this
│ TLS         │         ├─────────────┤
│ malloc arena│         │ No TLS here!│
└─────────────┘         └─────────────┘
The Fix
Don't allocate from GVThread stack!
Before (broken):
rustextern "C" fn gvthread_entry(closure_ptr: usize) {
    let token = CancellationToken::new();  // ← Arc::new() → malloc() → BOOM
    (*boxed)(&token);
}
After (fixed):
rustextern "C" fn gvthread_entry(closure_ptr: usize) {
    // Use metadata's cancelled field - NO ALLOCATION
    let token = CancellationToken::from_metadata(meta);
    (*boxed)(&token);
}
CancellationToken now has 3 variants:
rustenum CancellationInner {
    Owned(Arc<...>),           // Heap - for outside GVThread
    Metadata(*const AtomicU8), // Points to metadata - NO ALLOC
    Dummy,                     // Never cancels - NO ALLOC
}
Other Fixes Applied
IssueFixVec::push() in release()Pre-allocate free_stack to max_slotsBinaryHeap::push() in sleep queuePre-allocate to max_gvthreadsStale wake after slot reuseGeneration counter in metadatakdebug! from GVThread stackRemoved debug logging from hot paths
The Rule
NEVER allocate heap memory from code running on GVThread stack:

No Box::new(), Vec::new(), String::new()
No Arc::new(), Rc::new()
No format!(), .to_string()
No Vec::push() that might grow
No println!() (allocates internally)

Safe alternatives:

Pre-allocate before context switch
Use pointers to pre-existing data
Use stack-only types (atomics, raw pointers)
