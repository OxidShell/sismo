use std::fmt::Write as _;

use super::Summary;
use crate::groups::Group;
use crate::units::{Clock, Nanos, Percent};

/// Renders a summary as Markdown, ready for an issue or a PR.
pub(super) fn render(s: &Summary) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {} {} — sismo report\n", s.app, s.app_version);
    let _ = writeln!(
        out,
        "- Device: {} ({}){}",
        s.device,
        s.platform,
        if s.build.is_empty() {
            String::new()
        } else {
            format!(" · build {}", s.build)
        }
    );
    let _ = writeln!(
        out,
        "- Window: {} · {} events · {} dropped",
        Nanos(s.duration_ns),
        s.events,
        s.dropped_events
    );
    let f = &s.frames;
    let _ = writeln!(
        out,
        "- Frames: {} · p50 {} · p95 {} · max {} · {} over 16.6 ms",
        f.count,
        Nanos(f.p50_ns),
        Nanos(f.p95_ns),
        Nanos(f.max_ns),
        f.slow
    );
    let _ = writeln!(
        out,
        "- Time split: app {} · framework {} · other {}\n",
        Percent(s.split.percent(Group::App)),
        Percent(s.split.percent(Group::Framework)),
        Percent(s.split.percent(Group::Other)),
    );
    if !s.panics.is_empty() {
        let _ = writeln!(out, "## Panics\n");
        for p in &s.panics {
            let _ = writeln!(
                out,
                "- `{}` at {} on {}: {}",
                Clock(p.t_ns),
                p.location,
                p.thread,
                p.message
            );
        }
        out.push('\n');
    }
    let _ = writeln!(out, "## Top functions by self time\n");
    let _ = writeln!(
        out,
        "| Function | Crate | Group | Calls | Self | Total | p50 | p95 | Max |"
    );
    let _ = writeln!(out, "|---|---|---|---:|---:|---:|---:|---:|---:|");
    for row in &s.top_functions {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} |",
            row.name,
            row.krate,
            row.group,
            row.calls,
            Nanos(row.self_ns),
            Nanos(row.total_ns),
            Nanos(row.p50_ns),
            Nanos(row.p95_ns),
            Nanos(row.max_ns)
        );
    }
    if !s.slow_frames.is_empty() {
        let _ = writeln!(out, "\n## Slowest frames\n");
        for frame in &s.slow_frames {
            let top: Vec<String> = frame
                .top
                .iter()
                .map(|(name, ns)| format!("`{name}` {}", Nanos(*ns)))
                .collect();
            let _ = writeln!(
                out,
                "- #{} at {}: {} (app {} · framework {}) — {}",
                frame.index,
                Clock(frame.start_ns),
                Nanos(frame.duration_ns),
                Percent(frame.split.percent(Group::App)),
                Percent(frame.split.percent(Group::Framework)),
                top.join(", ")
            );
        }
    }
    let l = &s.logs;
    let _ = writeln!(
        out,
        "\n## Logs\n\n{} errors · {} warnings · {} info · {} debug · {} trace\n",
        l.error, l.warn, l.info, l.debug, l.trace
    );
    for line in &s.problems {
        let _ = writeln!(
            out,
            "- `{}` **{}** {} ({}): {}",
            Clock(line.t_ns),
            line.level,
            line.target,
            line.source,
            line.message
        );
    }
    if !s.markers.is_empty() {
        let _ = writeln!(out, "\n## Markers\n");
        for m in &s.markers {
            let _ = writeln!(out, "- `{}` {}", Clock(m.t_ns), m.note);
        }
    }
    out
}
