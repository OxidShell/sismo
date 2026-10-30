use std::fmt::Write as _;

use sismo_format::{Level, Source};
use tracing_core::field::{Field, Visit};
use tracing_core::span::{Attributes, Id};
use tracing_core::{Event, Metadata, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::registry::{DynamicKey, registry};
use crate::thread;

/// `tracing-subscriber` layer recording spans as sismo scopes and events as
/// logs. Compose it with your other layers:
///
/// ```no_run
/// use tracing_subscriber::layer::SubscriberExt;
/// let subscriber = tracing_subscriber::registry().with(sismo::SismoLayer::default());
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct SismoLayer;

#[derive(Debug, Clone, Copy)]
struct SpanCallsite(u32);

fn level(metadata: &Metadata<'_>) -> Level {
    match *metadata.level() {
        tracing_core::Level::ERROR => Level::Error,
        tracing_core::Level::WARN => Level::Warn,
        tracing_core::Level::INFO => Level::Info,
        tracing_core::Level::DEBUG => Level::Debug,
        tracing_core::Level::TRACE => Level::Trace,
    }
}

fn callsite(metadata: &Metadata<'_>, kind: u8) -> u32 {
    let key = DynamicKey {
        kind,
        name: metadata.name().to_owned(),
        target: metadata.target().to_owned(),
        file: metadata.file().unwrap_or_default().to_owned(),
        line: metadata.line().unwrap_or_default(),
    };
    registry().register_dynamic(
        key,
        metadata.module_path().unwrap_or(metadata.target()),
        level(metadata),
    )
}

#[derive(Default)]
struct Fields {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.fields
                .push((field.name().to_owned(), value.to_owned()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            self.fields
                .push((field.name().to_owned(), format!("{value:?}")));
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for SismoLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let metadata = attrs.metadata();
        if !crate::is_active() || !registry().allows_span(metadata.target()) {
            return;
        }
        if let Some(span) = ctx.span(id) {
            span.extensions_mut()
                .insert(SpanCallsite(callsite(metadata, 0)));
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(SpanCallsite(callsite)) = span.extensions().get::<SpanCallsite>().copied()
        {
            thread::record(|w, t| w.begin(t, callsite));
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && span.extensions().get::<SpanCallsite>().is_some()
        {
            thread::record(sismo_format::BatchWriter::end);
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let lvl = level(metadata);
        if !crate::is_active() || !registry().allows(metadata.target(), lvl) {
            return;
        }
        let callsite = callsite(metadata, 1);
        let mut fields = Fields::default();
        event.record(&mut fields);
        thread::record(|w, t| {
            w.log(
                t,
                callsite,
                lvl,
                Source::Tracing,
                &fields.message,
                fields.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
        });
    }
}
