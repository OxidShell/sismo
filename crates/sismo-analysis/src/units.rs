use std::fmt;

/// Nanoseconds rendered with a readable unit: `812 ns`, `4.21 ms`, `1.30 s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Nanos(pub u64);

impl fmt::Display for Nanos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[allow(clippy::cast_precision_loss)]
        let ns = self.0 as f64;
        match self.0 {
            0..1_000 => write!(f, "{} ns", self.0),
            1_000..1_000_000 => write!(f, "{:.1} µs", ns / 1e3),
            1_000_000..1_000_000_000 => write!(f, "{:.2} ms", ns / 1e6),
            1_000_000_000..60_000_000_000 => write!(f, "{:.2} s", ns / 1e9),
            _ => {
                let secs = self.0 / 1_000_000_000;
                write!(f, "{} min {:02} s", secs / 60, secs % 60)
            }
        }
    }
}

/// Session offset rendered as `mm:ss.mmm` (or `h:mm:ss.mmm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock(pub u64);

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.0 / 1_000_000;
        let (h, m, s, rest) = (
            ms / 3_600_000,
            (ms / 60_000) % 60,
            (ms / 1000) % 60,
            ms % 1000,
        );
        if h > 0 {
            write!(f, "{h}:{m:02}:{s:02}.{rest:03}")
        } else {
            write!(f, "{m:02}:{s:02}.{rest:03}")
        }
    }
}

/// Percentage with one decimal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percent(pub f64);

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}%", self.0)
    }
}

/// Byte count with a binary unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bytes(pub u64);

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[allow(clippy::cast_precision_loss)]
        let b = self.0 as f64;
        match self.0 {
            0..1024 => write!(f, "{} B", self.0),
            1024..1_048_576 => write!(f, "{:.1} KB", b / 1024.0),
            1_048_576..1_073_741_824 => write!(f, "{:.1} MB", b / 1_048_576.0),
            _ => write!(f, "{:.2} GB", b / 1_073_741_824.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_units() {
        assert_eq!(Nanos(812).to_string(), "812 ns");
        assert_eq!(Nanos(4_210_000).to_string(), "4.21 ms");
        assert_eq!(Nanos(125_000_000_000).to_string(), "2 min 05 s");
        assert_eq!(Clock(65_432_000_000).to_string(), "01:05.432");
        assert_eq!(Bytes(12_400_000).to_string(), "11.8 MB");
    }
}
