use std::fs::File;
use std::path::Path;

use sismo::format::{Event, FileReader, ReadOutcome, Record};

#[sismo::function]
fn step(n: u64) -> u64 {
    n + 1
}

fn records_in(dir: &Path) -> Vec<Record> {
    let file = std::fs::read_dir(dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    FileReader::new(File::open(file).unwrap())
        .unwrap()
        .flat_map(|outcome| match outcome {
            ReadOutcome::Chunk(records) => records,
            ReadOutcome::Damaged { error, .. } => panic!("damaged: {error}"),
        })
        .collect()
}

fn session(dir: &Path) -> Vec<Record> {
    let handle = sismo::install(sismo::config!().record_to(dir)).unwrap();
    for n in 0..10 {
        step(n);
    }
    handle.flush();
    handle.shutdown();
    assert!(sismo::handle().is_none());
    assert!(!sismo::is_active());
    records_in(dir)
}

#[test]
fn installs_again_after_shutdown_with_every_definition() {
    let root = std::env::temp_dir().join(format!("sismo-reinstall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let first = session(&root.join("first"));
    step(99);
    let second = session(&root.join("second"));

    for records in [&first, &second] {
        assert!(matches!(records[0], Record::Session(_)));
        let id = records
            .iter()
            .find_map(|r| match r {
                Record::Callsite(c) if c.name.ends_with("::step") => Some(c.id),
                _ => None,
            })
            .unwrap();
        let begins = records
            .iter()
            .flat_map(|r| match r {
                Record::Batch { events, .. } => events.clone(),
                _ => vec![],
            })
            .filter(|e| matches!(e, Event::Begin { callsite, .. } if *callsite == id))
            .count();
        assert_eq!(begins, 10);
    }
    let session_id = |records: &[Record]| match &records[0] {
        Record::Session(s) => s.session_id,
        _ => unreachable!(),
    };
    assert_ne!(session_id(&first), session_id(&second));
    let _ = std::fs::remove_dir_all(&root);
}
