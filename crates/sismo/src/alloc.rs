//! Counting global allocator.
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: sismo::alloc::Counting = sismo::alloc::Counting::system();
//! ```
//!
//! Every scope then reports the bytes and allocations made while it ran,
//! and the recorder samples the live heap as `alloc.live_bytes`.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static FREED: AtomicU64 = AtomicU64::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Bytes and allocations made by this thread so far.
    static THREAD: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// Wraps an allocator and counts what goes through it.
#[derive(Debug, Default)]
pub struct Counting<A = System> {
    inner: A,
}

impl Counting<System> {
    #[must_use]
    pub const fn system() -> Self {
        Self { inner: System }
    }
}

impl<A> Counting<A> {
    #[must_use]
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }
}

#[inline]
fn on_alloc(size: usize) {
    let size = size as u64;
    INSTALLED.store(true, Ordering::Relaxed);
    ALLOCATED.fetch_add(size, Ordering::Relaxed);
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    // `try_with` because the allocator runs during thread-local teardown too.
    let _ = THREAD.try_with(|c| {
        let (bytes, count) = c.get();
        c.set((bytes + size, count + 1));
    });
}

#[inline]
fn on_free(size: usize) {
    FREED.fetch_add(size as u64, Ordering::Relaxed);
}

// SAFETY: every method forwards to the wrapped allocator with the caller's
// arguments unchanged, so `A`'s guarantees carry over; the bookkeeping only
// touches atomics and a const-initialized thread-local `Cell`, which never
// allocate.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Counting<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded verbatim; the caller upholds `alloc`'s contract.
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            on_alloc(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded verbatim; the caller upholds `alloc_zeroed`'s contract.
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() {
            on_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim; `ptr` came from this allocator with `layout`.
        unsafe { self.inner.dealloc(ptr, layout) };
        on_free(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded verbatim; the caller upholds `realloc`'s contract.
        let new = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            on_free(layout.size());
            on_alloc(new_size);
        }
        new
    }
}

/// Whether a [`Counting`] allocator is in use.
#[must_use]
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// `(bytes, allocations)` made by the calling thread so far.
#[must_use]
pub fn thread_totals() -> (u64, u64) {
    THREAD.try_with(Cell::get).unwrap_or((0, 0))
}

/// Process-wide `(allocated bytes, freed bytes, allocations)` so far.
#[must_use]
pub fn totals() -> (u64, u64, u64) {
    (
        ALLOCATED.load(Ordering::Relaxed),
        FREED.load(Ordering::Relaxed),
        ALLOCATIONS.load(Ordering::Relaxed),
    )
}
