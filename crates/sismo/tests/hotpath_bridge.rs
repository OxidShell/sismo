//! hotpath scopes recorded through the function sink.
//!
//! `cargo test -p sismo --features hotpath --test hotpath_bridge`

use std::fs::File;

use sismo::format::{CallsiteDef, Event, FileReader, ReadOutcome, Record};

#[hotpath::measure]
fn inner(n: u64) -> u64 {
    std::hint::black_box(n * 2)
}

#[hotpath::measure]
fn outer(n: u64) -> u64 {
    inner(n) + inner(n + 1)
}

#[hotpath::measure]
async fn async_work() -> u32 {
    7
}

fn read_all(path: &std::path::Path) -> Vec<Record> {
    FileReader::new(File::open(path).unwrap())
        .unwrap()
        .flat_map(|outcome| match outcome {
            ReadOutcome::Chunk(records) => records,
            ReadOutcome::Damaged { error, .. } => panic!("damaged: {error}"),
        })
        .collect()
}

fn callsite<'a>(callsites: &'a [CallsiteDef], suffix: &str) -> &'a CallsiteDef {
    callsites
        .iter()
        .find(|c| c.name.ends_with(suffix))
        .unwrap_or_else(|| panic!("no callsite *{suffix}"))
}

#[test]
#[allow(
    unsafe_code,
    reason = "hotpath::measure_block! exports its label as a unique symbol"
)]
fn hotpath_scopes_become_spans() {
    let dir = std::env::temp_dir().join(format!("sismo-hotpath-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let handle = sismo::install(sismo::config!().filter("trace").record_to(&dir)).unwrap();

    for n in 0..10 {
        outer(n);
    }
    hotpath::measure_block!("hot_block", {
        std::hint::black_box(1);
    });
    assert_eq!(block_on(async_work()), 7);
    handle.flush();

    let file = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let records = read_all(&file);
    let callsites: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            Record::Callsite(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let events: Vec<_> = records
        .iter()
        .flat_map(|r| match r {
            Record::Batch { events, .. } => events.clone(),
            _ => vec![],
        })
        .collect();
    let begins = |id: u32| {
        events
            .iter()
            .filter(|e| matches!(e, Event::Begin { callsite, .. } if *callsite == id))
            .count()
    };

    let outer_cs = callsite(&callsites, "hotpath_bridge::outer");
    assert_eq!(outer_cs.crate_name(), "hotpath_bridge");
    assert_eq!(begins(outer_cs.id), 10);
    assert_eq!(begins(callsite(&callsites, "hotpath_bridge::inner").id), 20);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Begin { .. }))
            .count(),
        events
            .iter()
            .filter(|e| matches!(e, Event::End { .. }))
            .count(),
    );

    let block_cs = callsite(&callsites, "hot_block");
    assert_eq!(block_cs.module_path, "hotpath");
    let async_cs = callsite(&callsites, "hotpath_bridge::async_work");
    for cs in [block_cs, async_cs] {
        let id = events
            .iter()
            .find_map(|e| match e {
                Event::AsyncBegin { callsite, id, .. } if *callsite == cs.id => Some(*id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no async begin for {}", cs.name));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::AsyncEnd { id: end, .. } if *end == id))
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    let waker = Waker::from(Arc::new(Noop));
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return v;
        }
    }
}
