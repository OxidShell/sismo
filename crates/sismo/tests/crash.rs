use std::fs::File;

use sismo::format::{Event, FileReader, ReadOutcome, Record};

#[sismo::function]
fn parse_page(index: usize) -> u32 {
    let pages = [1u32, 2, 3];
    pages[index]
}

#[test]
fn a_thread_panic_is_recorded_with_its_backtrace_and_kept_as_a_crash_report() {
    let root = std::env::temp_dir().join(format!("sismo-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dumps = root.join("dumps");
    let handle =
        sismo::install(sismo::config!().flight_recorder(1 << 20).dump_dir(&dumps)).unwrap();

    let worker = std::thread::Builder::new()
        .name("importer".into())
        .spawn(|| parse_page(std::hint::black_box(7)))
        .unwrap();
    assert!(worker.join().is_err());

    let reports = handle.crash_reports();
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert!(report.name.ends_with("-panic.sismo"));

    let panic = FileReader::new(File::open(&report.path).unwrap())
        .unwrap()
        .flat_map(|outcome| match outcome {
            ReadOutcome::Chunk(records) => records,
            ReadOutcome::Damaged { error, .. } => panic!("damaged: {error}"),
        })
        .flat_map(|record| match record {
            Record::Batch { events, .. } => events,
            _ => Vec::new(),
        })
        .find_map(|event| match event {
            Event::Panic {
                message, backtrace, ..
            } => Some((message, backtrace)),
            _ => None,
        })
        .unwrap();
    assert!(panic.0.contains("index out of bounds"));
    assert!(panic.1.contains("parse_page"), "{}", panic.1);
    assert!(!panic.1.contains("core::panicking"), "{}", panic.1);

    handle.delete_crash_report(&report.name).unwrap();
    assert!(handle.crash_reports().is_empty());
    assert!(handle.delete_crash_report("../escape-panic.sismo").is_err());
    handle.shutdown();
    let _ = std::fs::remove_dir_all(root);
}
