//! Crash reports: flight recorder dumps written when the app panicked,
//! kept on disk until a viewer fetches or the app deletes them.

use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// File name suffix of a dump written on panic.
pub(crate) const SUFFIX: &str = "-panic.sismo";
/// Reports kept on disk; older ones are deleted when a new one is written.
const KEEP: usize = 5;

/// A crash report on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// File name, unique within the dump directory.
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// When it was written, in nanoseconds since the Unix epoch.
    pub written_unix_ns: u64,
}

impl From<&CrashReport> for sismo_format::net::CrashReport {
    fn from(report: &CrashReport) -> Self {
        Self {
            name: report.name.clone(),
            bytes: report.bytes,
            written_unix_ns: report.written_unix_ns,
        }
    }
}

/// Crash reports in `dir`, newest first; none when it cannot be read.
#[must_use]
pub fn crash_reports(dir: &Path) -> Vec<CrashReport> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut reports: Vec<CrashReport> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.ends_with(SUFFIX) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let written = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
            Some(CrashReport {
                name,
                path: entry.path(),
                bytes: meta.len(),
                written_unix_ns: u64::try_from(written.as_nanos()).unwrap_or(u64::MAX),
            })
        })
        .collect();
    reports.sort_unstable_by_key(|r| std::cmp::Reverse(r.written_unix_ns));
    reports
}

/// Path of the report `name` in `dir`, refusing anything that is not a
/// plain crash report file name.
pub(crate) fn report_path(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let plain = Path::new(name).file_name().is_some_and(|n| n == name);
    if !plain || !name.ends_with(SUFFIX) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a crash report name",
        ));
    }
    Ok(dir.join(name))
}

/// Deletes the report `name` from `dir`.
///
/// # Errors
/// `name` is not a crash report file name, or it cannot be removed.
pub fn delete_crash_report(dir: &Path, name: &str) -> io::Result<()> {
    std::fs::remove_file(report_path(dir, name)?)
}

/// Deletes all but the newest [`KEEP`] reports.
pub(crate) fn prune(dir: &Path) {
    for old in crash_reports(dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_report_names_resolve() {
        let dir = Path::new("/tmp/dumps");
        assert!(report_path(dir, "app-panic.sismo").is_ok());
        assert!(report_path(dir, "../app-panic.sismo").is_err());
        assert!(report_path(dir, "app-export.sismo").is_err());
        assert!(report_path(dir, "/etc/app-panic.sismo").is_err());
    }

    #[test]
    fn keeps_the_newest_reports() {
        let dir = std::env::temp_dir().join(format!("sismo-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..7 {
            std::fs::write(dir.join(format!("{i}{SUFFIX}")), b"x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        std::fs::write(dir.join("other.sismo"), b"x").unwrap();
        prune(&dir);
        let left: Vec<String> = crash_reports(&dir).into_iter().map(|r| r.name).collect();
        assert_eq!(left.len(), KEEP);
        assert_eq!(left[0], format!("6{SUFFIX}"));
        assert!(dir.join("other.sismo").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
