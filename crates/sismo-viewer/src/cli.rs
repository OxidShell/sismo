use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use sismo_analysis::units::{Bytes, Clock, Nanos, Percent};
use sismo_analysis::{
    ExportOptions, Format, Group, LoadError, Query, QueryError, Session, SortBy, SummaryOptions,
    parse_duration,
};
use sismo_format::{Chunk, FileWriter};

use crate::client::{ClientEvent, Command as Cmd, Connection, ScanEvent, Scanner};

/// Remote and recorded performance + log analysis for Rust apps.
///
/// Without a subcommand the desktop viewer opens.
#[derive(Debug, Parser)]
#[command(name = "sismo", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the viewer on a recording.
    Open {
        file: PathBuf,
        /// Diff it against this recording (shown in Funciones).
        #[arg(long)]
        compare: Option<PathBuf>,
    },
    /// Open the viewer connected to a live app (`ip:port` or app name).
    Connect { target: String },
    /// Overview: frames, time split, top functions, slow frames, problems.
    Summary(FileArgs),
    /// Per-function timing table.
    Functions {
        #[command(flatten)]
        file: FileArgs,
        /// self, total, calls, p95, max or name.
        #[arg(long, default_value = "self")]
        sort: String,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Slowest frames with the app/framework split.
    Frames {
        #[command(flatten)]
        file: FileArgs,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Log lines matching the query.
    Logs {
        #[command(flatten)]
        file: FileArgs,
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Convert a recording to another format.
    Export {
        #[command(flatten)]
        file: FileArgs,
        /// perfetto, chrome, json, md or sismo.
        #[arg(long, short = 'f', default_value = "perfetto")]
        format: String,
        /// Output path (defaults next to the input).
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        no_logs: bool,
        /// Drop structured log fields (may carry user data).
        #[arg(long)]
        strip_fields: bool,
    },
    /// Function-by-function diff between two recordings (B relative to A).
    Compare {
        a: PathBuf,
        b: PathBuf,
        /// p95, p50, self or calls.
        #[arg(long, default_value = "p95")]
        metric: String,
        #[arg(long, short = 'q', default_value = "")]
        query: String,
        #[arg(long, default_value_t = 30)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// List apps streaming sismo on the LAN.
    Scan {
        #[arg(long, default_value = "3s")]
        timeout: String,
    },
    /// Record a live app to a file.
    Record {
        #[command(flatten)]
        live: LiveArgs,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Stop after this long (e.g. 30s); runs until the app leaves otherwise.
        #[arg(long)]
        duration: Option<String>,
        /// Skip the app's ring-buffer history.
        #[arg(long)]
        no_history: bool,
    },
    /// Follow a live app's logs.
    Tail {
        #[command(flatten)]
        live: LiveArgs,
        #[arg(long, short = 'q', default_value = "")]
        query: String,
    },
}

#[derive(Debug, Args)]
pub struct FileArgs {
    /// A `.sismo` recording.
    pub file: PathBuf,
    /// Query in the viewer syntax, e.g. "crate:app dur>2ms".
    #[arg(long, short = 'q', default_value = "")]
    pub query: String,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LiveArgs {
    /// `ip:port`, or an app name to find with a scan.
    pub target: String,
    /// Pairing code shown by the app (asked for when needed).
    #[arg(long)]
    pub code: Option<String>,
    /// Give up when the connection drops instead of reconnecting.
    #[arg(long)]
    pub no_reconnect: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error("invalid query: {0}")]
    Query(#[from] QueryError),
    #[error("unknown {what} `{value}`")]
    Unknown { what: &'static str, value: String },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Live(String),
}

fn load(args: &FileArgs) -> Result<(Session, Query), CliError> {
    let (session, report) = Session::load(&args.file)?;
    if let Some((offset, error)) = report.damaged {
        eprintln!(
            "warning: {} is cut at byte {offset} ({error}); showing what could be read",
            args.file.display()
        );
    }
    Ok((session, Query::parse(&args.query)?))
}

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}

/// Runs a headless subcommand.
///
/// # Errors
/// [`CliError`] for unreadable input, bad arguments or network failures.
pub fn run(command: Command) -> Result<(), CliError> {
    match command {
        Command::Open { .. } | Command::Connect { .. } => Ok(()),
        Command::Summary(args) => summary(&args),
        Command::Functions { file, sort, limit } => functions(&file, &sort, limit),
        Command::Frames { file, limit } => frames(&file, limit),
        Command::Logs { file, limit } => logs(&file, limit),
        Command::Export {
            file,
            format,
            output,
            no_logs,
            strip_fields,
        } => export(&file, &format, output, !no_logs, strip_fields),
        Command::Compare {
            a,
            b,
            metric,
            query,
            limit,
            json,
        } => compare(&a, &b, &metric, &query, limit, json),
        Command::Scan { timeout } => scan(&timeout),
        Command::Record {
            live,
            output,
            duration,
            no_history,
        } => record(&live, output, duration.as_deref(), !no_history),
        Command::Tail { live, query } => tail(&live, &query),
    }
}

fn summary(args: &FileArgs) -> Result<(), CliError> {
    let (session, query) = load(args)?;
    let summary = session.summary(&SummaryOptions {
        query,
        ..SummaryOptions::default()
    });
    if args.json {
        return print_json(&summary);
    }
    let mut out = Vec::new();
    session.export(
        Format::Markdown,
        &ExportOptions {
            query: Query::parse(&args.query)?,
            include_logs: true,
            ..ExportOptions::default()
        },
        &mut out,
    )?;
    io::stdout().write_all(&out)?;
    Ok(())
}

fn functions(args: &FileArgs, sort: &str, limit: usize) -> Result<(), CliError> {
    let (session, query) = load(args)?;
    let sort = SortBy::parse(sort).ok_or_else(|| CliError::Unknown {
        what: "sort",
        value: sort.to_owned(),
    })?;
    let mut rows = session.functions(&query, query.window(&session), 12, sort);
    rows.truncate(limit);
    if args.json {
        return print_json(&rows);
    }
    println!(
        "{:<56} {:<10} {:>8} {:>11} {:>11} {:>10} {:>10} {:>10} {:>10}",
        "function", "group", "calls", "self", "total", "p50", "p95", "max", "alloc"
    );
    for row in rows {
        println!(
            "{:<56} {:<10} {:>8} {:>11} {:>11} {:>10} {:>10} {:>10} {:>10}",
            truncate(&row.name, 56),
            row.group.as_str(),
            row.calls,
            Nanos(row.self_ns).to_string(),
            Nanos(row.total_ns).to_string(),
            Nanos(row.p50_ns).to_string(),
            Nanos(row.p95_ns).to_string(),
            Nanos(row.max_ns).to_string(),
            if session.allocations {
                Bytes(row.alloc_self_bytes).to_string()
            } else {
                "—".to_owned()
            },
        );
    }
    Ok(())
}

fn frames(args: &FileArgs, limit: usize) -> Result<(), CliError> {
    let (session, query) = load(args)?;
    let window = query.window(&session);
    let summary = session.summary(&SummaryOptions {
        query,
        window: Some(window),
        slow_frames: limit,
        ..SummaryOptions::default()
    });
    if args.json {
        return print_json(&summary.slow_frames);
    }
    let f = summary.frames;
    println!(
        "{} frames · p50 {} · p95 {} · max {} · {} over 16.6 ms",
        f.count,
        Nanos(f.p50_ns),
        Nanos(f.p95_ns),
        Nanos(f.max_ns),
        f.slow
    );
    for frame in summary.slow_frames {
        println!(
            "#{:<7} {}  {:>10}  app {:>6}  framework {:>6}  {}",
            frame.index,
            Clock(frame.start_ns),
            Nanos(frame.duration_ns).to_string(),
            Percent(frame.split.percent(Group::App)).to_string(),
            Percent(frame.split.percent(Group::Framework)).to_string(),
            frame
                .top
                .iter()
                .take(3)
                .map(|(n, ns)| format!("{n} {}", Nanos(*ns)))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

fn logs(args: &FileArgs, limit: usize) -> Result<(), CliError> {
    let (session, query) = load(args)?;
    let (from, to) = query.window(&session);
    let lines: Vec<_> = session
        .logs
        .iter()
        .filter(|l| l.t >= from && l.t < to && query.matches_log(&session, l))
        .take(limit)
        .collect();
    if args.json {
        let json: Vec<_> = lines
            .iter()
            .map(|l| {
                serde_json::json!({
                    "t_ns": l.t,
                    "level": l.level.as_str(),
                    "source": l.source.as_str(),
                    "target": session.callsite(l.callsite).map(|c| c.target.clone()),
                    "thread": session.thread_name(l.thread),
                    "message": l.message,
                    "fields": l.fields,
                })
            })
            .collect();
        return print_json(&json);
    }
    for log in lines {
        println!("{}", log_line(&session, log));
    }
    Ok(())
}

fn log_line(session: &Session, log: &sismo_analysis::LogEntry) -> String {
    let target = session
        .callsite(log.callsite)
        .map_or("?", |c| c.target.as_str());
    let fields: Vec<String> = log.fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!(
        "{} {:<5} {:<7} {target}: {}{}",
        Clock(log.t),
        log.level.as_str().to_uppercase(),
        log.source.as_str(),
        log.message,
        if fields.is_empty() {
            String::new()
        } else {
            format!("  {}", fields.join(" "))
        }
    )
}

fn export(
    args: &FileArgs,
    format: &str,
    output: Option<PathBuf>,
    include_logs: bool,
    strip_fields: bool,
) -> Result<(), CliError> {
    let format = Format::parse(format).ok_or_else(|| CliError::Unknown {
        what: "format",
        value: format.to_owned(),
    })?;
    let (session, query) = load(args)?;
    let output = output.unwrap_or_else(|| {
        let stem = args.file.file_stem().unwrap_or_default().to_string_lossy();
        let suffix = if format == Format::Sismo {
            "-trimmed"
        } else {
            ""
        };
        args.file
            .with_file_name(format!("{stem}{suffix}.{}", format.extension()))
    });
    let mut out = BufWriter::new(File::create(&output)?);
    session.export(
        format,
        &ExportOptions {
            query,
            window: None,
            include_logs,
            strip_fields,
        },
        &mut out,
    )?;
    out.flush()?;
    println!("{}", output.display());
    Ok(())
}

fn scan(timeout: &str) -> Result<(), CliError> {
    let timeout = Duration::from_nanos(parse_duration(timeout)?);
    let scanner = Scanner::start(Duration::from_millis(500))?;
    let deadline = Instant::now() + timeout;
    let mut found = 0;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match scanner.events.recv_timeout(left) {
            Ok(Ok(ScanEvent::Found(app))) => {
                found += 1;
                let b = &app.beacon;
                println!(
                    "{:<22} {} {} · {} {} · {:?} · key {}",
                    app.addr.to_string(),
                    b.app,
                    b.app_version,
                    b.platform,
                    b.device,
                    b.auth,
                    b.fingerprint
                );
            }
            Ok(Ok(ScanEvent::Lost(_))) => {}
            Ok(Err(error)) => return Err(CliError::Live(error)),
            Err(_) => break,
        }
    }
    if found == 0 {
        eprintln!("no app found; is it running with sismo in live mode on this network?");
    }
    Ok(())
}

pub fn resolve(target: &str) -> Result<SocketAddr, CliError> {
    if let Ok(addr) = target.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let scanner = Scanner::start(Duration::from_millis(500))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match scanner.events.recv_timeout(left) {
            Ok(Ok(ScanEvent::Found(app))) if app.beacon.app == target => return Ok(app.addr),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Err(CliError::Live(format!(
        "no app named `{target}` found on the LAN"
    )))
}

fn ask_code() -> Result<String, CliError> {
    eprint!("pairing code shown by the app: ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

/// What a [`LiveLink`] hands back.
enum Received {
    Chunk(Vec<u8>),
    HistoryEnd,
    /// The app is a new run (it restarted): what came before belongs to the
    /// old one.
    Restarted,
}

/// A connection to a live app that reconnects by itself (backoff 1–30 s,
/// finding the app again by name) and asks only for what it missed.
struct LiveLink<'a> {
    live: &'a LiveArgs,
    history: bool,
    conn: Connection,
    session_id: u64,
    last_seq: u64,
}

impl<'a> LiveLink<'a> {
    fn open(live: &'a LiveArgs, history: bool) -> Result<Self, CliError> {
        let mut link = Self {
            live,
            history,
            conn: Self::dial(live)?,
            session_id: 0,
            last_seq: 0,
        };
        link.handshake()?;
        Ok(link)
    }

    fn dial(live: &LiveArgs) -> Result<Connection, CliError> {
        let addr = resolve(&live.target)?;
        let viewer = format!("sismo-cli {}", env!("CARGO_PKG_VERSION"));
        Ok(Connection::open(addr, viewer)?)
    }

    /// Waits for the welcome, pairs when asked and subscribes. Returns
    /// whether the app is a different run than the one before.
    fn handshake(&mut self) -> Result<bool, CliError> {
        let mut restarted = false;
        loop {
            match self.conn.events.recv_timeout(Duration::from_secs(10)) {
                Ok(ClientEvent::Welcome {
                    session,
                    pairing_required,
                    ..
                }) => {
                    if self.session_id == 0 {
                        eprintln!(
                            "connected to {} {} on {} ({})",
                            session.app, session.app_version, session.device, session.platform
                        );
                    } else if session.session_id != self.session_id {
                        eprintln!("the app restarted");
                        restarted = true;
                        self.last_seq = 0;
                    } else {
                        eprintln!("reconnected");
                    }
                    self.session_id = session.session_id;
                    if !pairing_required {
                        break;
                    }
                    let code = match &self.live.code {
                        Some(code) => code.clone(),
                        None => ask_code()?,
                    };
                    self.conn.send(Cmd::Pair(code));
                }
                Ok(ClientEvent::Paired) => break,
                Ok(ClientEvent::PairRejected(reason) | ClientEvent::Lost(reason)) => {
                    return Err(CliError::Live(reason));
                }
                Ok(_) => {}
                Err(_) => return Err(CliError::Live("the app did not answer".into())),
            }
        }
        self.conn.send(Cmd::Subscribe {
            history: self.history || self.last_seq > 0,
            since: self.last_seq,
        });
        Ok(restarted)
    }

    /// Next item, waiting up to `timeout`; `Ok(None)` on timeout. A lost
    /// connection is retried until it comes back, unless reconnecting is off.
    fn next(&mut self, timeout: Duration) -> Result<Option<Received>, CliError> {
        loop {
            match self.conn.events.recv_timeout(timeout) {
                Ok(ClientEvent::Chunk { seq, bytes }) => {
                    if seq != 0 && seq <= self.last_seq {
                        continue;
                    }
                    self.last_seq = self.last_seq.max(seq);
                    return Ok(Some(Received::Chunk(bytes)));
                }
                Ok(ClientEvent::HistoryEnd) => return Ok(Some(Received::HistoryEnd)),
                Ok(ClientEvent::Lost(reason)) => {
                    if self.live.no_reconnect {
                        return Err(CliError::Live(reason));
                    }
                    eprintln!("lost: {reason}; reconnecting…");
                    if self.reconnect()? {
                        return Ok(Some(Received::Restarted));
                    }
                }
                Ok(_) => {}
                Err(flume::RecvTimeoutError::Timeout) => return Ok(None),
                Err(flume::RecvTimeoutError::Disconnected) => {
                    return Err(CliError::Live("connection closed".into()));
                }
            }
        }
    }

    fn reconnect(&mut self) -> Result<bool, CliError> {
        let mut delay = Duration::from_secs(1);
        loop {
            std::thread::sleep(delay);
            delay = (delay * 2).min(Duration::from_secs(30));
            let Ok(conn) = Self::dial(self.live) else {
                continue;
            };
            self.conn = conn;
            match self.handshake() {
                Ok(restarted) => return Ok(restarted),
                Err(CliError::Live(reason)) => eprintln!("still lost: {reason}"),
                Err(error) => return Err(error),
            }
        }
    }
}

fn record(
    live: &LiveArgs,
    output: Option<PathBuf>,
    duration: Option<&str>,
    history: bool,
) -> Result<(), CliError> {
    let stop_after = duration
        .map(parse_duration)
        .transpose()?
        .map(Duration::from_nanos);
    let mut link = LiveLink::open(live, history)?;
    let output =
        output.unwrap_or_else(|| PathBuf::from(format!("{}.sismo", live.target.replace(':', "-"))));
    let mut path = output.clone();
    let mut writer = FileWriter::new(BufWriter::new(File::create(&path)?))?;
    let mut runs = 1u32;
    let started = Instant::now();
    let mut chunks = 0u64;
    loop {
        let left = stop_after.map_or(Duration::from_secs(1), |d| {
            d.saturating_sub(started.elapsed())
        });
        if stop_after.is_some() && left.is_zero() {
            break;
        }
        match link.next(left.min(Duration::from_secs(1))) {
            Ok(Some(Received::Chunk(bytes))) => {
                if let Ok(chunk) = Chunk::from_bytes(bytes) {
                    writer.write_chunk(&chunk)?;
                    writer.flush()?;
                    chunks += 1;
                }
            }
            Ok(Some(Received::HistoryEnd)) => eprintln!("history received, recording live…"),
            Ok(Some(Received::Restarted)) => {
                // One file per run of the app: mixing two runs would break
                // the timeline.
                writer.flush()?;
                eprintln!("{} → {}", Bytes(writer.bytes_written()), path.display());
                runs += 1;
                path = output.with_file_name(format!(
                    "{}-run{runs}.sismo",
                    output
                        .file_stem()
                        .map_or_else(|| "recording".into(), |s| s.to_string_lossy().into_owned())
                ));
                writer = FileWriter::new(BufWriter::new(File::create(&path)?))?;
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("stopped: {error}");
                break;
            }
        }
    }
    writer.flush()?;
    eprintln!(
        "{} chunks · {} → {}",
        chunks,
        Bytes(writer.bytes_written()),
        path.display()
    );
    Ok(())
}

fn tail(live: &LiveArgs, query: &str) -> Result<(), CliError> {
    let query = Query::parse(query)?;
    let mut link = LiveLink::open(live, false)?;
    let mut session = Session::default();
    let mut printed = 0usize;
    loop {
        match link.next(Duration::from_secs(1))? {
            Some(Received::Chunk(bytes)) => {
                if session.ingest_chunk(bytes).is_err() {
                    continue;
                }
                for log in &session.logs[printed..] {
                    if query.matches_log(&session, log) {
                        println!("{}", log_line(&session, log));
                    }
                }
                printed = session.logs.len();
            }
            Some(Received::Restarted) => {
                println!("--- the app restarted ---");
                session = Session::default();
                printed = 0;
            }
            Some(Received::HistoryEnd) | None => {}
        }
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(width - 1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

fn compare(
    a: &std::path::Path,
    b: &std::path::Path,
    metric: &str,
    query: &str,
    limit: usize,
    json: bool,
) -> Result<(), CliError> {
    let metric = sismo_analysis::Metric::parse(metric).ok_or_else(|| CliError::Unknown {
        what: "metric",
        value: metric.to_owned(),
    })?;
    let query = Query::parse(query)?;
    let (left, _) = Session::load(a)?;
    let (right, _) = Session::load(b)?;
    let mut rows = sismo_analysis::compare(&left, &right, &query, metric);
    rows.truncate(limit);
    if json {
        return print_json(&rows);
    }
    let show = |v: Option<u64>| match (v, metric) {
        (None, _) => "—".to_owned(),
        (Some(v), sismo_analysis::Metric::Calls) => v.to_string(),
        (Some(v), _) => Nanos(v).to_string(),
    };
    println!(
        "A {}  ·  B {}  ·  {}",
        a.display(),
        b.display(),
        metric.label()
    );
    println!(
        "{:<56} {:>11} {:>11} {:>11} {:>8}",
        "function", "A", "B", "Δ", "Δ %"
    );
    for row in rows {
        let delta = match (row.delta, metric) {
            (None, _) => {
                if row.a.is_none() {
                    "new".to_owned()
                } else {
                    "gone".to_owned()
                }
            }
            (Some(d), sismo_analysis::Metric::Calls) => format!("{d:+}"),
            (Some(d), _) => format!(
                "{}{}",
                if d < 0 { "-" } else { "+" },
                Nanos(d.unsigned_abs())
            ),
        };
        let percent = row
            .percent
            .map_or_else(String::new, |p| format!("{p:+.0}%"));
        println!(
            "{:<56} {:>11} {:>11} {:>11} {:>8}{}",
            truncate(&row.name, 56),
            show(row.a),
            show(row.b),
            delta,
            percent,
            if row.is_noise() { "  ~" } else { "" }
        );
    }
    Ok(())
}
