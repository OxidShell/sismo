use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use crate::registry::{ACTIVE, Callsite};
use crate::thread::record;

static NEXT_ASYNC_ID: AtomicU64 = AtomicU64::new(1);

/// Records a span from creation until drop on the current thread.
#[derive(Debug)]
#[must_use = "the span ends when the guard is dropped"]
pub struct ScopeGuard {
    active: bool,
}

impl ScopeGuard {
    #[doc(hidden)]
    #[inline]
    pub fn enter(callsite: &'static Callsite) -> Self {
        if !ACTIVE.load(Ordering::Relaxed) || !callsite.enabled() {
            return Self { active: false };
        }
        let id = callsite.id();
        record(|w, t| w.begin(t, id));
        Self { active: true }
    }
}

impl Drop for ScopeGuard {
    #[inline]
    fn drop(&mut self) {
        if self.active {
            record(sismo_format::BatchWriter::end);
        }
    }
}

/// Future wrapper that records one async span from first poll to
/// completion, independent of the threads it is polled on.
#[must_use = "futures do nothing unless polled"]
pub struct Instrumented<F> {
    inner: Pin<Box<F>>,
    callsite: &'static Callsite,
    id: u64,
    started: bool,
    finished: bool,
}

impl<F> Instrumented<F> {
    #[doc(hidden)]
    pub fn new(callsite: &'static Callsite, inner: F) -> Self {
        Self {
            inner: Box::pin(inner),
            callsite,
            id: 0,
            started: false,
            finished: false,
        }
    }
}

impl<F: Future> Future for Instrumented<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = &mut *self;
        if !this.started {
            this.started = true;
            if ACTIVE.load(Ordering::Relaxed) && this.callsite.enabled() {
                let callsite = this.callsite.id();
                let id = NEXT_ASYNC_ID.fetch_add(1, Ordering::Relaxed);
                this.id = id;
                record(|w, t| w.async_begin(t, callsite, id));
            }
        }
        let poll = this.inner.as_mut().poll(cx);
        if poll.is_ready() && this.id != 0 && !this.finished {
            this.finished = true;
            let id = this.id;
            record(|w, t| w.async_end(t, id));
        }
        poll
    }
}

impl<F> Drop for Instrumented<F> {
    fn drop(&mut self) {
        if self.id != 0 && !self.finished {
            let id = self.id;
            record(|w, t| w.async_end(t, id));
        }
    }
}
