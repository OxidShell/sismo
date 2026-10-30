use std::fs::File;

use sismo::format::{Event, FileReader, ReadOutcome, Record};

#[sismo::function]
fn leaf(n: u64) -> u64 {
    sismo::scope!("leaf_inner");
    n * 2
}

#[sismo::function]
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

#[test]
fn records_spans_counters_logs_and_dumps() {
    let dir = std::env::temp_dir().join(format!("sismo-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let handle = sismo::install(
        sismo::config!()
            .filter("trace")
            .record_to(dir.join("rec"))
            .dump_dir(dir.join("dump")),
    )
    .unwrap();
    sismo::log::init(None, log::LevelFilter::Off).unwrap();

    let workers: Vec<_> = (0..3)
        .map(|i| {
            std::thread::Builder::new()
                .name(format!("worker-{i}"))
                .spawn(move || {
                    for n in 0..100 {
                        leaf(n);
                    }
                    sismo::counter!("progress", 100);
                    sismo::frame_mark();
                })
                .unwrap()
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    log::warn!(target: "demo", free_mb = 12; "disk almost full");
    assert_eq!(pollster_block_on(async_work()), 7);
    sismo::marker("checkpoint");
    handle.flush();

    let rec_dir = dir.join("rec");
    let file = std::fs::read_dir(&rec_dir)
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
    let leaf_cs = callsites
        .iter()
        .find(|c| c.name.ends_with("::leaf"))
        .unwrap();
    assert_eq!(leaf_cs.crate_name(), "record");
    let events: Vec<_> = records
        .iter()
        .flat_map(|r| match r {
            Record::Batch { events, .. } => events.clone(),
            _ => vec![],
        })
        .collect();
    let leaf_begins = events
        .iter()
        .filter(|e| matches!(e, Event::Begin { callsite, .. } if *callsite == leaf_cs.id))
        .count();
    assert_eq!(leaf_begins, 300);
    let begins = events
        .iter()
        .filter(|e| matches!(e, Event::Begin { .. }))
        .count();
    let ends = events
        .iter()
        .filter(|e| matches!(e, Event::End { .. }))
        .count();
    assert_eq!(begins, ends);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Frame { .. }))
            .count(),
        3
    );
    assert!(events.iter().any(
        |e| matches!(e, Event::Counter { value, .. } if (*value - 100.0).abs() < f64::EPSILON)
    ));
    assert!(events.iter().any(|e| matches!(e, Event::Log { message, fields, .. } if message == "disk almost full" && fields[0].0 == "free_mb")));
    assert!(events.iter().any(|e| matches!(e, Event::AsyncBegin { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::AsyncEnd { .. })));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Marker { note, .. } if note == "checkpoint"))
    );
    assert!(
        records
            .iter()
            .any(|r| matches!(r, Record::Thread(t) if t.name == "worker-1"))
    );

    let dump = handle.dump_flight_recorder("test").unwrap();
    let dumped = read_all(&dump);
    assert!(matches!(dumped[0], Record::Session(_)));
    assert!(dumped.iter().any(|r| matches!(r, Record::Batch { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

fn pollster_block_on<F: std::future::Future>(future: F) -> F::Output {
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
