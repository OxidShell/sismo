mod chrome;
mod perfetto;
mod report;
mod summary;
mod trimmed;

pub use summary::{LogCounts, LogLine, SlowFrame, Summary, SummaryOptions};

use std::io::{self, Write};

use crate::query::Query;
use crate::session::Session;

/// Output formats for [`Session::export`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Perfetto protobuf trace (`.pftrace`), opens in ui.perfetto.dev.
    Perfetto,
    /// Chrome trace event JSON.
    Chrome,
    /// Machine-readable summary (`.json`) for agents and scripts.
    SummaryJson,
    /// Human-readable Markdown report.
    Markdown,
    /// A `.sismo` recording limited to the window.
    Sismo,
}

impl Format {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "perfetto" | "pftrace" => Some(Format::Perfetto),
            "chrome" => Some(Format::Chrome),
            "json" | "summary" => Some(Format::SummaryJson),
            "md" | "markdown" | "report" => Some(Format::Markdown),
            "sismo" => Some(Format::Sismo),
            _ => None,
        }
    }

    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Format::Perfetto => "pftrace",
            Format::Chrome | Format::SummaryJson => "json",
            Format::Markdown => "md",
            Format::Sismo => "sismo",
        }
    }
}

/// Export settings shared by every format.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub query: Query,
    /// Absolute window; the whole session when `None`.
    pub window: Option<(u64, u64)>,
    pub include_logs: bool,
    /// Drop structured log fields (they may carry user data).
    pub strip_fields: bool,
}

impl Session {
    /// Writes the session in `format`.
    ///
    /// # Errors
    /// I/O errors from `out`.
    pub fn export(
        &self,
        format: Format,
        options: &ExportOptions,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let window = options.window.unwrap_or_else(|| options.query.window(self));
        match format {
            Format::Perfetto => perfetto::write(self, options, window, out),
            Format::Chrome => chrome::write(self, options, window, out),
            Format::SummaryJson => {
                let summary = self.summary(&SummaryOptions {
                    query: options.query.clone(),
                    window: Some(window),
                    ..SummaryOptions::default()
                });
                serde_json::to_writer_pretty(&mut *out, &summary).map_err(io::Error::other)?;
                out.write_all(b"\n")
            }
            Format::Markdown => {
                let summary = self.summary(&SummaryOptions {
                    query: options.query.clone(),
                    window: Some(window),
                    ..SummaryOptions::default()
                });
                out.write_all(report::render(&summary).as_bytes())
            }
            Format::Sismo => trimmed::write(self, options, window, out),
        }
    }
}
