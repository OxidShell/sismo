use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Instant;

use sismo_format::{CallsiteDef, CallsiteKind, Filter, Level, Record, SessionInfo, ThreadDef};

/// Whether the recorder accepts events. Checked first on every hot path.
pub(crate) static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Bumped whenever the filter changes so cached callsite decisions refresh.
static FILTER_GENERATION: AtomicU32 = AtomicU32::new(1);

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub(crate) fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

/// Monotonic clock shared by every thread.
pub(crate) fn now() -> u64 {
    let start = registry().start;
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DynamicKey {
    pub kind: u8,
    pub name: String,
    pub target: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Default)]
struct Definitions {
    session: SessionInfo,
    threads: Vec<ThreadDef>,
    callsites: Vec<CallsiteDef>,
    pending: Vec<Record>,
}

/// Interning table for threads and callsites plus the active filter.
#[derive(Debug)]
pub(crate) struct Registry {
    start: Instant,
    defs: Mutex<Definitions>,
    dynamic: RwLock<HashMap<DynamicKey, u32>>,
    filter: RwLock<Filter>,
    next_callsite: AtomicU32,
    next_thread: AtomicU32,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            defs: Mutex::default(),
            dynamic: RwLock::default(),
            filter: RwLock::new(Filter::default()),
            next_callsite: AtomicU32::new(1),
            next_thread: AtomicU32::new(1),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section is a short push; recover instead of propagating poison.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Registry {
    /// Starts a session: the next collector receives it followed by every
    /// thread and callsite known so far, so a reinstall keeps its names.
    pub fn set_session(&self, session: SessionInfo) {
        let mut defs = lock(&self.defs);
        defs.session = session;
        let mut pending = Vec::with_capacity(1 + defs.threads.len() + defs.callsites.len());
        pending.push(Record::Session(defs.session.clone()));
        pending.extend(defs.threads.iter().cloned().map(Record::Thread));
        pending.extend(defs.callsites.iter().cloned().map(Record::Callsite));
        defs.pending = pending;
    }

    pub fn session(&self) -> SessionInfo {
        lock(&self.defs).session.clone()
    }

    pub fn register_thread(&self, name: String) -> u32 {
        let id = self.next_thread.fetch_add(1, Ordering::Relaxed);
        let def = ThreadDef { id, name };
        let mut defs = lock(&self.defs);
        defs.pending.push(Record::Thread(def.clone()));
        defs.threads.push(def);
        id
    }

    fn insert_callsite(&self, mut def: CallsiteDef) -> u32 {
        let id = self.next_callsite.fetch_add(1, Ordering::Relaxed);
        def.id = id;
        let mut defs = lock(&self.defs);
        defs.pending.push(Record::Callsite(def.clone()));
        defs.callsites.push(def);
        id
    }

    pub fn register_static(&self, callsite: &Callsite) -> u32 {
        let def = CallsiteDef {
            id: 0,
            kind: callsite.kind,
            name: callsite.name.to_owned(),
            target: callsite.target.to_owned(),
            module_path: callsite.module_path.to_owned(),
            file: callsite.file.to_owned(),
            line: callsite.line,
            level: callsite.level,
        };
        let id = self.insert_callsite(def);
        match callsite
            .id
            .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => id,
            // Another thread won the race; its id is the canonical one and ours stays unused.
            Err(existing) => existing,
        }
    }

    /// Interns a callsite known only at runtime (log records, tracing
    /// metadata, stdio lines).
    pub fn register_dynamic(&self, key: DynamicKey, module_path: &str, level: Level) -> u32 {
        if let Some(id) = self
            .dynamic
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return *id;
        }
        let mut map = self
            .dynamic
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = map.get(&key) {
            return *id;
        }
        let kind = match key.kind {
            0 => CallsiteKind::Span,
            2 => CallsiteKind::Counter,
            _ => CallsiteKind::Event,
        };
        let id = self.insert_callsite(CallsiteDef {
            id: 0,
            kind,
            name: key.name.clone(),
            target: key.target.clone(),
            module_path: module_path.to_owned(),
            file: key.file.clone(),
            line: key.line,
            level,
        });
        map.insert(key, id);
        id
    }

    /// Records emitted since the last call, in registration order.
    pub fn take_pending(&self) -> Vec<Record> {
        std::mem::take(&mut lock(&self.defs).pending)
    }

    /// Every definition known so far, for a new file or viewer.
    pub fn snapshot(&self) -> Vec<Record> {
        let defs = lock(&self.defs);
        let mut records = Vec::with_capacity(1 + defs.threads.len() + defs.callsites.len());
        records.push(Record::Session(defs.session.clone()));
        records.extend(defs.threads.iter().cloned().map(Record::Thread));
        records.extend(defs.callsites.iter().cloned().map(Record::Callsite));
        records
    }

    pub fn set_filter(&self, filter: Filter) {
        *self
            .filter
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = filter;
        FILTER_GENERATION.fetch_add(1, Ordering::Release);
    }

    pub fn filter(&self) -> Filter {
        self.filter
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn allows(&self, target: &str, level: Level) -> bool {
        self.filter
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .allows(target, level)
    }

    pub fn allows_span(&self, target: &str) -> bool {
        self.filter
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .allows_span(target)
    }
}

/// A static instrumentation point, created by the `scope!`, `counter!` and
/// `#[function]` macros. Not meant to be built by hand.
#[derive(Debug)]
pub struct Callsite {
    id: AtomicU32,
    /// `generation << 1 | enabled`, refreshed when the filter changes.
    state: AtomicU32,
    kind: CallsiteKind,
    name: &'static str,
    target: &'static str,
    module_path: &'static str,
    file: &'static str,
    line: u32,
    level: Level,
}

impl Callsite {
    #[doc(hidden)]
    #[must_use]
    pub const fn new(
        kind: CallsiteKind,
        name: &'static str,
        module_path: &'static str,
        file: &'static str,
        line: u32,
    ) -> Self {
        Self {
            id: AtomicU32::new(0),
            state: AtomicU32::new(0),
            kind,
            name,
            target: module_path,
            module_path,
            file,
            line,
            level: Level::Trace,
        }
    }

    pub(crate) fn id(&self) -> u32 {
        match self.id.load(Ordering::Relaxed) {
            0 => registry().register_static(self),
            id => id,
        }
    }

    /// Whether the current filter lets this callsite record.
    pub(crate) fn enabled(&self) -> bool {
        let generation = FILTER_GENERATION.load(Ordering::Acquire);
        let state = self.state.load(Ordering::Relaxed);
        if state >> 1 == generation {
            return state & 1 == 1;
        }
        let enabled = registry().allows_span(self.target);
        self.state
            .store((generation << 1) | u32::from(enabled), Ordering::Relaxed);
        enabled
    }
}
