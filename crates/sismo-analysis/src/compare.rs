use std::collections::HashMap;

use serde::Serialize;

use crate::groups::Group;
use crate::query::Query;
use crate::session::Session;
use crate::stats::{FunctionStats, SortBy};

/// Statistic compared between two recordings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Metric {
    #[default]
    P95,
    P50,
    SelfTime,
    Calls,
}

impl Metric {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "p95" => Some(Metric::P95),
            "p50" => Some(Metric::P50),
            "self" => Some(Metric::SelfTime),
            "calls" => Some(Metric::Calls),
            _ => None,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Metric::P95 => "p95",
            Metric::P50 => "p50",
            Metric::SelfTime => "self",
            Metric::Calls => "calls",
        }
    }

    fn of(self, row: &FunctionStats) -> u64 {
        match self {
            Metric::P95 => row.p95_ns,
            Metric::P50 => row.p50_ns,
            Metric::SelfTime => row.self_ns,
            Metric::Calls => row.calls,
        }
    }
}

/// One function present in either recording.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareRow {
    pub name: String,
    #[serde(rename = "crate")]
    pub krate: String,
    pub group: Group,
    pub a: Option<u64>,
    pub b: Option<u64>,
    /// `b - a` when both exist.
    pub delta: Option<i64>,
    /// Relative change in percent when both exist and `a > 0`.
    pub percent: Option<f64>,
}

impl CompareRow {
    /// Changes inside this band count as noise.
    pub const NOISE_PERCENT: f64 = 10.0;

    #[must_use]
    pub fn is_noise(&self) -> bool {
        self.percent.is_some_and(|p| p.abs() <= Self::NOISE_PERCENT)
    }
}

/// Matches functions by name and compares `metric`, largest change first.
#[must_use]
pub fn compare(a: &Session, b: &Session, query: &Query, metric: Metric) -> Vec<CompareRow> {
    let rows = |s: &Session| -> HashMap<String, FunctionStats> {
        s.functions(query, query.window(s), 1, SortBy::Name)
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect()
    };
    let (left, right) = (rows(a), rows(b));
    let mut names: Vec<&String> = left.keys().chain(right.keys()).collect();
    names.sort();
    names.dedup();
    let mut out: Vec<CompareRow> = names
        .into_iter()
        .filter_map(|name| {
            let (l, r) = (left.get(name), right.get(name));
            let any = l.or(r)?;
            let av = l.map(|row| metric.of(row));
            let bv = r.map(|row| metric.of(row));
            let delta = av.zip(bv).map(|(x, y)| {
                i64::try_from(y).unwrap_or(i64::MAX) - i64::try_from(x).unwrap_or(i64::MAX)
            });
            #[allow(clippy::cast_precision_loss)]
            let percent = av
                .zip(delta)
                .filter(|(x, _)| *x > 0)
                .map(|(x, d)| d as f64 / x as f64 * 100.);
            Some(CompareRow {
                name: name.clone(),
                krate: any.krate.clone(),
                group: any.group,
                a: av,
                b: bv,
                delta,
                percent,
            })
        })
        .collect();
    out.sort_by_key(|row| std::cmp::Reverse(row.delta.map_or(u64::MAX, i64::unsigned_abs)));
    out
}
