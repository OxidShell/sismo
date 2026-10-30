use regex_lite::Regex;
use sismo_format::{Level, Source};

use crate::groups::Group;
use crate::session::{LogEntry, Session, Span};

/// Frames slower than this count as slow (60 Hz budget).
pub const SLOW_FRAME_NS: u64 = 16_666_667;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("`{key}` needs a value")]
    MissingValue { key: String },
    #[error("bad duration `{0}` (use ns, us, ms or s)")]
    BadDuration(String),
    #[error("bad number `{0}`")]
    BadNumber(String),
    #[error("unknown level `{0}`")]
    BadLevel(String),
    #[error("unknown source `{0}`")]
    BadSource(String),
    #[error("bad regex `{0}`")]
    BadRegex(String),
    #[error("unterminated quote")]
    UnterminatedQuote,
}

/// Every key the query language understands, with a one-line help and an
/// example (shown by suggestion popups and `--help`).
pub const KEYS: &[(&str, &str, &str)] = &[
    (
        "crate:",
        "crate or group (app, framework, other)",
        "crate:freya_*",
    ),
    ("fn:", "function name; * globs and /regex/", "fn:*paint*"),
    ("dur>", "span duration (also dur<, dur>=, dur<=)", "dur>2ms"),
    ("self>", "self time of a span or function", "self>1ms"),
    ("calls>", "call count in the functions table", "calls>100"),
    ("level:", "log level; + includes more severe", "level:warn+"),
    ("thread:", "thread name", "thread:main"),
    ("frame:", "frame number or slow", "frame:slow"),
    ("source:", "tracing, log, stdout, stderr", "source:stdout"),
    ("file:", "source file path", "file:canvas.rs"),
    ("range:", "last N of the session, or +a..+b", "range:5m"),
    ("find:", "highlight without filtering", "find:\"gpu\""),
];

#[derive(Debug, Clone)]
pub enum Matcher {
    Exact(String),
    Contains(String),
    Glob(String),
    Regex(Regex),
}

impl PartialEq for Matcher {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Matcher::Exact(a), Matcher::Exact(b))
            | (Matcher::Contains(a), Matcher::Contains(b))
            | (Matcher::Glob(a), Matcher::Glob(b)) => a == b,
            (Matcher::Regex(a), Matcher::Regex(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

impl Matcher {
    fn parse(value: &str, loose: bool) -> Result<Self, QueryError> {
        if value.len() >= 2 && value.starts_with('/') && value.ends_with('/') {
            let pattern = &value[1..value.len() - 1];
            return Regex::new(pattern)
                .map(Matcher::Regex)
                .map_err(|_| QueryError::BadRegex(pattern.to_owned()));
        }
        let lower = value.to_lowercase();
        Ok(if lower.contains('*') {
            Matcher::Glob(lower)
        } else if loose {
            Matcher::Contains(lower)
        } else {
            Matcher::Exact(lower)
        })
    }

    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        match self {
            Matcher::Exact(v) => {
                text.eq_ignore_ascii_case(v) || text.replace('-', "_").eq_ignore_ascii_case(v)
            }
            Matcher::Contains(v) => text.to_lowercase().contains(v.as_str()),
            Matcher::Glob(pattern) => glob(pattern, &text.to_lowercase()),
            Matcher::Regex(re) => re.is_match(text),
        }
    }

    /// Byte ranges of matches inside `text`, for highlighting.
    #[must_use]
    pub fn ranges(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            Matcher::Contains(v) | Matcher::Exact(v) if !v.is_empty() => {
                let lower = text.to_lowercase();
                if lower.len() != text.len() {
                    return Vec::new();
                }
                lower
                    .match_indices(v.as_str())
                    .map(|(i, m)| (i, i + m.len()))
                    .collect()
            }
            Matcher::Regex(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
            _ => Vec::new(),
        }
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            let Some(stripped) = rest.strip_prefix(part) else {
                return false;
            };
            rest = stripped;
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    parts
        .last()
        .is_none_or(|last| last.is_empty() || text.ends_with(last))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
}

impl Cmp {
    fn test(self, a: u64, b: u64) -> bool {
        match self {
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameSel {
    Index(u32),
    Slow,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TermKind {
    Crate(Vec<Matcher>, Vec<Group>),
    Function(Vec<Matcher>),
    Thread(Vec<Matcher>),
    File(Vec<Matcher>),
    Level { level: Level, or_worse: bool },
    Source(Vec<Source>),
    Duration(Cmp, u64),
    SelfTime(Cmp, u64),
    Calls(Cmp, u64),
    Frame(FrameSel),
    Text(Matcher),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub negate: bool,
    pub kind: TermKind,
}

/// Relative time window of a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// The last `n` nanoseconds of the session.
    Last(u64),
    /// Offsets from the session start.
    Between(u64, u64),
}

/// Parsed query: OR-ed groups of AND-ed terms, plus highlight-only
/// `find:` terms and an optional time window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub groups: Vec<Vec<Term>>,
    pub find: Vec<Matcher>,
    pub range: Option<RangeSpec>,
}

/// Parses `12ms`, `3.5s`, `800us`, `40ns`; a bare number means ms.
///
/// # Errors
/// [`QueryError::BadDuration`].
pub fn parse_duration(value: &str) -> Result<u64, QueryError> {
    let value = value.trim();
    let split = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| QueryError::BadDuration(value.to_owned()))?;
    let scale = match unit {
        "ns" => 1.0,
        "us" | "µs" => 1e3,
        "" | "ms" => 1e6,
        "s" => 1e9,
        "m" | "min" => 60e9,
        _ => return Err(QueryError::BadDuration(value.to_owned())),
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((number * scale) as u64)
}

fn tokenize(input: &str) -> Result<Vec<String>, QueryError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if quoted {
        return Err(QueryError::UnterminatedQuote);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

fn split_cmp(token: &str) -> Option<(&str, Cmp, &str)> {
    for (op, cmp) in [
        (">=", Cmp::Ge),
        ("<=", Cmp::Le),
        (">", Cmp::Gt),
        ("<", Cmp::Lt),
    ] {
        if let Some(pos) = token.find(op) {
            let key = &token[..pos];
            if key.chars().all(|c| c.is_ascii_alphabetic()) && !key.is_empty() {
                return Some((key, cmp, &token[pos + op.len()..]));
            }
        }
    }
    None
}

fn values(value: &str, loose: bool) -> Result<Vec<Matcher>, QueryError> {
    value
        .split(',')
        .filter(|v| !v.is_empty())
        .map(|v| Matcher::parse(unquote(v), loose))
        .collect()
}

impl Query {
    /// # Errors
    /// [`QueryError`] describing the first invalid term.
    pub fn parse(input: &str) -> Result<Self, QueryError> {
        let mut query = Query::default();
        let mut group = Vec::new();
        for token in tokenize(input)? {
            if token == "OR" {
                if !group.is_empty() {
                    query.groups.push(std::mem::take(&mut group));
                }
                continue;
            }
            let (negate, body) = match token.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, token.as_str()),
            };
            if let Some((key, cmp, value)) = split_cmp(body) {
                let kind = match key {
                    "dur" | "duration" => TermKind::Duration(cmp, parse_duration(value)?),
                    "self" => TermKind::SelfTime(cmp, parse_duration(value)?),
                    "calls" => TermKind::Calls(
                        cmp,
                        value
                            .parse()
                            .map_err(|_| QueryError::BadNumber(value.to_owned()))?,
                    ),
                    other => return Err(QueryError::UnknownKey(other.to_owned())),
                };
                group.push(Term { negate, kind });
                continue;
            }
            let Some((key, value)) = body
                .split_once(':')
                .filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphabetic()))
            else {
                group.push(Term {
                    negate,
                    kind: TermKind::Text(Matcher::parse(unquote(body), true)?),
                });
                continue;
            };
            if value.is_empty() {
                return Err(QueryError::MissingValue {
                    key: key.to_owned(),
                });
            }
            let kind = match key {
                "crate" => {
                    let mut matchers = Vec::new();
                    let mut groups = Vec::new();
                    for part in value.split(',').filter(|v| !v.is_empty()) {
                        match Group::parse(part) {
                            Some(g) => groups.push(g),
                            None => matchers.push(Matcher::parse(unquote(part), false)?),
                        }
                    }
                    TermKind::Crate(matchers, groups)
                }
                "fn" | "function" => TermKind::Function(values(value, true)?),
                "thread" => TermKind::Thread(values(value, true)?),
                "file" => TermKind::File(values(value, true)?),
                "level" => {
                    let (name, or_worse) = value
                        .strip_suffix('+')
                        .map_or((value, false), |v| (v, true));
                    TermKind::Level {
                        level: Level::parse(name)
                            .ok_or_else(|| QueryError::BadLevel(name.to_owned()))?,
                        or_worse,
                    }
                }
                "source" => TermKind::Source(
                    value
                        .split(',')
                        .map(|v| {
                            Source::parse(v).ok_or_else(|| QueryError::BadSource(v.to_owned()))
                        })
                        .collect::<Result<_, _>>()?,
                ),
                "frame" => TermKind::Frame(match value {
                    "slow" | "lento" | "lentos" => FrameSel::Slow,
                    number => FrameSel::Index(
                        number
                            .trim_start_matches('#')
                            .parse()
                            .map_err(|_| QueryError::BadNumber(number.to_owned()))?,
                    ),
                }),
                "range" => {
                    query.range = Some(parse_range(value)?);
                    continue;
                }
                "find" => {
                    query.find.push(Matcher::parse(unquote(value), true)?);
                    continue;
                }
                other => return Err(QueryError::UnknownKey(other.to_owned())),
            };
            group.push(Term { negate, kind });
        }
        if !group.is_empty() {
            query.groups.push(group);
        }
        Ok(query)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.find.is_empty() && self.range.is_none()
    }

    /// Absolute `[from, to)` window for this session.
    #[must_use]
    pub fn window(&self, session: &Session) -> (u64, u64) {
        let (start, end) = session.span();
        match self.range {
            None => (start, end.saturating_add(1)),
            Some(RangeSpec::Last(n)) => (end.saturating_sub(n).max(start), end.saturating_add(1)),
            Some(RangeSpec::Between(a, b)) => (start + a, (start + b).min(end.saturating_add(1))),
        }
    }

    fn eval(&self, mut test: impl FnMut(&TermKind) -> Option<bool>) -> bool {
        if self.groups.is_empty() {
            return true;
        }
        self.groups.iter().any(|group| {
            group.iter().all(|term| match test(&term.kind) {
                Some(result) => result != term.negate,
                None => true,
            })
        })
    }

    fn crate_matches(
        session: &Session,
        callsite: u32,
        matchers: &[Matcher],
        groups: &[Group],
    ) -> bool {
        let krate = session.crate_of(callsite);
        matchers.iter().any(|m| m.matches(krate)) || groups.contains(&session.group_of(callsite))
    }

    fn frame_matches(session: &Session, sel: &FrameSel, t: u64) -> bool {
        let index = session.frames.partition_point(|f| f.end <= t);
        let Some(frame) = session.frames.get(index).filter(|f| f.start <= t) else {
            return false;
        };
        match sel {
            FrameSel::Index(i) => frame.index == *i,
            FrameSel::Slow => frame.duration() > SLOW_FRAME_NS,
        }
    }

    /// Whether a span passes the filter terms (log-only terms are ignored).
    #[must_use]
    pub fn matches_span(&self, session: &Session, thread: &str, span: &Span) -> bool {
        let callsite = span.callsite;
        self.eval(|kind| match kind {
            TermKind::Crate(m, g) => Some(Self::crate_matches(session, callsite, m, g)),
            TermKind::Function(m) => Some(m.iter().any(|m| m.matches(session.name(callsite)))),
            TermKind::Thread(m) => Some(m.iter().any(|m| m.matches(thread))),
            TermKind::File(m) => Some(
                session
                    .callsite(callsite)
                    .is_some_and(|c| m.iter().any(|m| m.matches(&c.file))),
            ),
            TermKind::Duration(cmp, ns) => Some(cmp.test(span.duration(session.end), *ns)),
            TermKind::SelfTime(cmp, ns) => Some(cmp.test(span.self_ns, *ns)),
            TermKind::Frame(sel) => Some(Self::frame_matches(session, sel, span.start)),
            TermKind::Text(m) => Some(m.matches(session.name(callsite))),
            TermKind::Level { .. } | TermKind::Source(_) | TermKind::Calls(..) => None,
        })
    }

    /// Whether a log entry passes the filter terms (span-only terms are ignored).
    #[must_use]
    pub fn matches_log(&self, session: &Session, log: &LogEntry) -> bool {
        self.eval(|kind| match kind {
            TermKind::Crate(m, g) => Some(Self::crate_matches(session, log.callsite, m, g)),
            TermKind::Thread(m) => {
                Some(m.iter().any(|m| m.matches(session.thread_name(log.thread))))
            }
            TermKind::File(m) => Some(
                session
                    .callsite(log.callsite)
                    .is_some_and(|c| m.iter().any(|m| m.matches(&c.file))),
            ),
            TermKind::Level { level, or_worse } => Some(if *or_worse {
                log.level <= *level
            } else {
                log.level == *level
            }),
            TermKind::Source(sources) => Some(sources.contains(&log.source)),
            TermKind::Frame(sel) => Some(Self::frame_matches(session, sel, log.t)),
            TermKind::Text(m) => Some(
                m.matches(&log.message)
                    || session
                        .callsite(log.callsite)
                        .is_some_and(|c| m.matches(&c.target))
                    || log.fields.iter().any(|(k, v)| m.matches(k) || m.matches(v)),
            ),
            TermKind::Function(_)
            | TermKind::Duration(..)
            | TermKind::SelfTime(..)
            | TermKind::Calls(..) => None,
        })
    }

    /// Whether a callsite can match at all (pre-filter for tables).
    #[must_use]
    pub fn matches_callsite(&self, session: &Session, callsite: u32) -> bool {
        self.eval(|kind| match kind {
            TermKind::Crate(m, g) => Some(Self::crate_matches(session, callsite, m, g)),
            TermKind::Function(m) => Some(m.iter().any(|m| m.matches(session.name(callsite)))),
            TermKind::File(m) => Some(
                session
                    .callsite(callsite)
                    .is_some_and(|c| m.iter().any(|m| m.matches(&c.file))),
            ),
            TermKind::Text(m) => Some(m.matches(session.name(callsite))),
            _ => None,
        })
    }

    /// Terms that apply to aggregated function rows (`calls`, `self`).
    #[must_use]
    pub fn matches_stats(&self, calls: u64, self_ns: u64, total_ns: u64) -> bool {
        self.eval(|kind| match kind {
            TermKind::Calls(cmp, n) => Some(cmp.test(calls, *n)),
            TermKind::SelfTime(cmp, ns) => Some(cmp.test(self_ns, *ns)),
            TermKind::Duration(cmp, ns) => Some(cmp.test(total_ns, *ns)),
            _ => None,
        })
    }

    /// Highlight ranges from `find:` terms and free text.
    #[must_use]
    pub fn highlights(&self, text: &str) -> Vec<(usize, usize)> {
        let text_terms = self.groups.iter().flatten().filter_map(|t| match &t.kind {
            TermKind::Text(m) if !t.negate => Some(m),
            _ => None,
        });
        let mut ranges: Vec<_> = self
            .find
            .iter()
            .chain(text_terms)
            .flat_map(|m| m.ranges(text))
            .collect();
        ranges.sort_unstable();
        ranges
    }
}

fn parse_range(value: &str) -> Result<RangeSpec, QueryError> {
    if let Some((a, b)) = value.split_once("..") {
        let a = parse_duration(a.trim_start_matches('+'))?;
        let b = parse_duration(b.trim_start_matches('+'))?;
        return Ok(RangeSpec::Between(a.min(b), a.max(b)));
    }
    Ok(RangeSpec::Last(parse_duration(
        value.trim_start_matches('-'),
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terms_groups_and_find() {
        let q = Query::parse(
            r#"crate:app,freya_* dur>2ms -thread:rayon OR level:warn+ find:"gpu budget" range:5m"#,
        )
        .unwrap();
        assert_eq!(q.groups.len(), 2);
        assert_eq!(q.groups[0].len(), 3);
        assert!(q.groups[0][2].negate);
        assert!(matches!(
            q.groups[0][1].kind,
            TermKind::Duration(Cmp::Gt, 2_000_000)
        ));
        assert_eq!(q.find.len(), 1);
        assert_eq!(q.range, Some(RangeSpec::Last(300_000_000_000)));
        assert!(matches!(
            q.groups[1][0].kind,
            TermKind::Level {
                level: Level::Warn,
                or_worse: true
            }
        ));
    }

    #[test]
    fn matchers_and_errors() {
        assert!(
            Matcher::parse("freya_*", false)
                .unwrap()
                .matches("freya_core")
        );
        assert!(
            Matcher::parse("*paint*", true)
                .unwrap()
                .matches("app::canvas::paint_ink")
        );
        assert!(
            Matcher::parse("/^torin::/", true)
                .unwrap()
                .matches("torin::measure")
        );
        assert!(!glob("a*b", "acb_x"));
        assert_eq!(parse_duration("1.5s").unwrap(), 1_500_000_000);
        assert_eq!(
            Query::parse("bogus:1"),
            Err(QueryError::UnknownKey("bogus".into()))
        );
        assert_eq!(
            Query::parse("dur>fast").unwrap_err(),
            QueryError::BadDuration("fast".into())
        );
        assert_eq!(
            Matcher::parse("gpu", true).unwrap().ranges("GPU and gpu"),
            vec![(0, 3), (8, 11)]
        );
    }
}
