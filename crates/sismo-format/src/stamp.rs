use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Nanoseconds since the Unix epoch, or zero when the clock is before it.
#[must_use]
pub fn unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

/// UTC timestamp rendered as `YYYYMMDD-HHMMSS`, for file names.
#[derive(Debug, Clone, Copy)]
pub struct FileStamp {
    unix_secs: u64,
}

impl FileStamp {
    #[must_use]
    pub fn now() -> Self {
        Self::from_unix_secs(unix_ns() / 1_000_000_000)
    }

    #[must_use]
    pub const fn from_unix_secs(unix_secs: u64) -> Self {
        Self { unix_secs }
    }

    /// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    fn civil(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (year, month as u32, day as u32)
    }
}

impl fmt::Display for FileStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.unix_secs;
        let days = i64::try_from(secs / 86_400).unwrap_or(0);
        let rem = secs % 86_400;
        let (year, month, day) = Self::civil(days);
        write!(
            f,
            "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
            rem / 3600,
            (rem % 3600) / 60,
            rem % 60
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instant() {
        let stamp = FileStamp {
            unix_secs: 1_793_318_853,
        };
        assert_eq!(stamp.to_string(), "20261030-000733");
    }

    #[test]
    fn file_stem_names_app_version_platform_device_and_time() {
        let info = crate::SessionInfo {
            app: "notinplus".into(),
            app_version: "0.1.0".into(),
            platform: "android".into(),
            device: "Lenovo TB373FU".into(),
            ..crate::SessionInfo::default()
        };
        assert_eq!(
            info.file_stem(FileStamp::from_unix_secs(1_793_318_853)),
            "notinplus-0.1.0-android-Lenovo_TB373FU-20261030-000733"
        );
    }
}
