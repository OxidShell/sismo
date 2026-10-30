use std::fmt;

use crate::Level;

/// Maximum level a target may record; `Off` disables it entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LevelFilter {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl LevelFilter {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LevelFilter::Off => "off",
            LevelFilter::Error => "error",
            LevelFilter::Warn => "warn",
            LevelFilter::Info => "info",
            LevelFilter::Debug => "debug",
            LevelFilter::Trace => "trace",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("off") {
            return Some(LevelFilter::Off);
        }
        Level::parse(value).map(LevelFilter::from)
    }

    #[must_use]
    pub fn allows(self, level: Level) -> bool {
        level as u8 <= self as u8
    }
}

impl From<Level> for LevelFilter {
    fn from(level: Level) -> Self {
        match level {
            Level::Error => LevelFilter::Error,
            Level::Warn => LevelFilter::Warn,
            Level::Info => LevelFilter::Info,
            Level::Debug => LevelFilter::Debug,
            Level::Trace => LevelFilter::Trace,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    #[error("unknown level `{0}`")]
    UnknownLevel(String),
    #[error("empty target before `=`")]
    EmptyTarget,
}

/// `RUST_LOG`-style directive list: `info,my_app=trace,zudp=off`.
///
/// The longest matching target prefix wins; spans are recorded unless their
/// target resolves to `off`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    default: LevelFilter,
    directives: Vec<(String, LevelFilter)>,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            default: LevelFilter::Info,
            directives: Vec::new(),
        }
    }
}

impl Filter {
    /// # Errors
    /// [`FilterError`] for an unknown level or an empty target.
    pub fn parse(spec: &str) -> Result<Self, FilterError> {
        let mut filter = Filter {
            default: LevelFilter::Info,
            directives: Vec::new(),
        };
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((target, level)) => {
                    let target = target.trim();
                    if target.is_empty() {
                        return Err(FilterError::EmptyTarget);
                    }
                    let level = LevelFilter::parse(level.trim())
                        .ok_or_else(|| FilterError::UnknownLevel(level.trim().to_owned()))?;
                    filter.set(target, level);
                }
                None => {
                    filter.default = LevelFilter::parse(part)
                        .ok_or_else(|| FilterError::UnknownLevel(part.to_owned()))?;
                }
            }
        }
        Ok(filter)
    }

    #[must_use]
    pub fn with_default(mut self, level: LevelFilter) -> Self {
        self.default = level;
        self
    }

    pub fn set(&mut self, target: &str, level: LevelFilter) {
        let target = target.replace('-', "_");
        if let Some(entry) = self.directives.iter_mut().find(|(t, _)| *t == target) {
            entry.1 = level;
        } else {
            self.directives.push((target, level));
            self.directives
                .sort_by_key(|d| std::cmp::Reverse(d.0.len()));
        }
    }

    #[must_use]
    pub fn default_level(&self) -> LevelFilter {
        self.default
    }

    pub fn directives(&self) -> impl Iterator<Item = (&str, LevelFilter)> {
        self.directives.iter().map(|(t, l)| (t.as_str(), *l))
    }

    /// Effective maximum level for `target`.
    #[must_use]
    pub fn level_for(&self, target: &str) -> LevelFilter {
        self.directives
            .iter()
            .find(|(prefix, _)| matches_prefix(target, prefix))
            .map_or(self.default, |(_, level)| *level)
    }

    #[must_use]
    pub fn allows(&self, target: &str, level: Level) -> bool {
        self.level_for(target).allows(level)
    }

    #[must_use]
    pub fn allows_span(&self, target: &str) -> bool {
        self.level_for(target) != LevelFilter::Off
    }

    /// Most verbose level any directive can let through.
    #[must_use]
    pub fn max_level(&self) -> LevelFilter {
        self.directives
            .iter()
            .map(|(_, l)| *l)
            .fold(self.default, LevelFilter::max)
    }
}

fn matches_prefix(target: &str, prefix: &str) -> bool {
    let normalized = target.replace('-', "_");
    normalized == prefix
        || normalized
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with("::"))
}

impl fmt::Display for Filter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.default.as_str())?;
        for (target, level) in &self.directives {
            write!(f, ",{target}={}", level.as_str())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins() {
        let filter = Filter::parse("warn,my_app=trace,my_app::noisy=off,zudp=error").unwrap();
        assert!(filter.allows("my_app::canvas", Level::Trace));
        assert!(!filter.allows("my_app::noisy::deep", Level::Error));
        assert!(!filter.allows_span("my_app::noisy"));
        assert!(filter.allows("other", Level::Warn));
        assert!(!filter.allows("other", Level::Info));
        assert!(!filter.allows("my_application", Level::Debug));
        assert!(filter.allows("my-app", Level::Trace));
        assert_eq!(filter.max_level(), LevelFilter::Trace);
    }

    #[test]
    fn display_round_trips() {
        let filter = Filter::parse("info, a=debug ,b::c=off").unwrap();
        assert_eq!(Filter::parse(&filter.to_string()).unwrap(), filter);
        assert_eq!(
            Filter::parse("verbose"),
            Err(FilterError::UnknownLevel("verbose".into()))
        );
    }
}
