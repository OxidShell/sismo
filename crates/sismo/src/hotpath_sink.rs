//! Bridge from hotpath's function sink: every `#[hotpath::measure]` and
//! `hotpath::measure_block!` scope becomes a sismo span, so code already
//! instrumented for hotpath shows up on the timeline without new macros.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use hotpath::{FunctionSink, ScopeKind};
use sismo_format::{BatchWriter, CallsiteKind, Level};

use crate::registry::{ACTIVE, DynamicKey, filter_generation, registry};
use crate::scope::next_async_id;
use crate::thread::record;

/// Module path given to `measure_block!` labels, which carry none.
const BLOCK_MODULE: &str = "hotpath";

/// Token of a scope that recorded nothing, so its exit records nothing.
/// Async span ids start at 1, so a recorded scope never returns it.
const SKIPPED: u64 = 0;
/// Token of a recorded sync scope; its end needs no id.
const RECORDED: u64 = 1;

#[derive(Debug, Clone, Copy)]
struct Interned {
    callsite: u32,
    module: &'static str,
    generation: u32,
    enabled: bool,
}

thread_local! {
    /// hotpath names are `&'static str`, so their address and length are a
    /// stable key: the registry's string-keyed lookup runs once per name and
    /// thread instead of on every call.
    static CALLSITES: RefCell<HashMap<(usize, usize), Interned>> = RefCell::new(HashMap::new());
    /// Names of the sync scopes open on this thread, innermost last.
    static OPEN_SYNC: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// The module path of `crate::module::function`. A `measure_block!` label
/// carries none, so it takes the module of the innermost measured function
/// open on this thread (`Element Render` inside `freya_core::render_pipeline`),
/// or `hotpath` outside of one. The first context a label is seen in wins.
/// The callsite keeps the full name, like `#[sismo::function]`.
fn module_path(name: &'static str) -> &'static str {
    let module_of = |name: &'static str| name.rsplit_once("::").map(|(module, _)| module);
    module_of(name)
        .or_else(|| {
            OPEN_SYNC
                .try_with(|open| open.try_borrow().ok()?.last().copied().and_then(module_of))
                .ok()
                .flatten()
        })
        .unwrap_or(BLOCK_MODULE)
}

fn intern(name: &'static str, generation: u32) -> Interned {
    let module = module_path(name);
    let callsite = registry().register_dynamic(
        DynamicKey {
            kind: CallsiteKind::Span as u8,
            name: name.to_owned(),
            target: module.to_owned(),
            file: String::new(),
            line: 0,
        },
        module,
        Level::Trace,
    );
    Interned {
        callsite,
        module,
        generation,
        enabled: registry().allows_span(module),
    }
}

/// The callsite of `name`, or `None` when the filter excludes its module.
fn callsite(name: &'static str) -> Option<u32> {
    let generation = filter_generation();
    CALLSITES
        .try_with(|cache| {
            let mut cache = cache.try_borrow_mut().ok()?;
            let entry = cache
                .entry((name.as_ptr().addr(), name.len()))
                .or_insert_with(|| intern(name, generation));
            if entry.generation != generation {
                entry.generation = generation;
                entry.enabled = registry().allows_span(entry.module);
            }
            entry.enabled.then_some(entry.callsite)
        })
        .ok()
        .flatten()
}

/// Sync scopes nest on the thread that entered them, so they map to plain
/// begin/end. Async scopes may resume on another thread or interleave with
/// other tasks, so they map to async spans paired by id.
#[derive(Debug, Default)]
struct Sink;

impl FunctionSink for Sink {
    fn enter(&self, name: &'static str, kind: ScopeKind) -> u64 {
        if kind == ScopeKind::Sync {
            OPEN_SYNC
                .try_with(|open| open.try_borrow_mut().map(|mut open| open.push(name)))
                .ok();
        }
        if !ACTIVE.load(Ordering::Relaxed) {
            return SKIPPED;
        }
        let Some(callsite) = callsite(name) else {
            return SKIPPED;
        };
        match kind {
            ScopeKind::Sync => {
                record(|w, t| w.begin(t, callsite));
                RECORDED
            }
            ScopeKind::Async => {
                let id = next_async_id();
                record(|w, t| w.async_begin(t, callsite, id));
                id
            }
            // A kind newer than this bridge: skip it rather than guess its nesting.
            _ => SKIPPED,
        }
    }

    fn exit(&self, _name: &'static str, kind: ScopeKind, token: u64) {
        if kind == ScopeKind::Sync {
            OPEN_SYNC
                .try_with(|open| open.try_borrow_mut().map(|mut open| open.pop()))
                .ok();
        }
        if token == SKIPPED {
            return;
        }
        match kind {
            ScopeKind::Sync => record(BatchWriter::end),
            ScopeKind::Async => record(|w, t| w.async_end(t, token)),
            _ => {}
        }
    }
}

/// Registers the sink with hotpath. hotpath keeps the first sink for the
/// whole process, so installing sismo again after a shutdown reuses it; the sink
/// itself checks whether sismo is recording.
pub(crate) fn install() {
    hotpath::set_function_sink(Sink).ok();
}
