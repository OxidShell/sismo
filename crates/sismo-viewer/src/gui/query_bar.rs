//! Query input with key and value suggestions.

use freya::prelude::*;
use sismo_analysis::{Group, KEYS, Session};
use sismo_format::{Level, Source};

use super::Workspace;
use super::i18n::tr;
use super::theme;
use super::widgets::{bordered, caption, mono, swatch, text};

/// One suggestion row: what to insert and how to describe it.
#[derive(Debug, Clone, PartialEq)]
struct Suggestion {
    insert: String,
    title: String,
    detail: String,
    dot: Option<theme::Rgb>,
}

fn last_token(text: &str) -> &str {
    text.rsplit(char::is_whitespace).next().unwrap_or("")
}

fn values(session: &Session, key: &str, prefix: &str) -> Vec<Suggestion> {
    let prefix_lower = prefix.to_lowercase();
    let matches = |v: &str| v.to_lowercase().contains(&prefix_lower);
    let mut out: Vec<Suggestion> = match key {
        "crate" => {
            let mut list: Vec<Suggestion> = Group::ALL
                .iter()
                .filter(|g| matches(g.as_str()))
                .map(|g| Suggestion {
                    insert: g.as_str().to_owned(),
                    title: g.as_str().to_owned(),
                    detail: tr!("query-group"),
                    dot: Some(theme::group(*g)),
                })
                .collect();
            list.extend(
                session
                    .crates()
                    .into_iter()
                    .filter(|c| matches(c))
                    .map(|c| {
                        let group = session.groups.classify(&c);
                        Suggestion {
                            detail: theme::group_label(group),
                            dot: Some(theme::group(group)),
                            insert: c.clone(),
                            title: c,
                        }
                    }),
            );
            list
        }
        "thread" => session
            .threads
            .iter()
            .filter(|t| matches(&t.name))
            .map(|t| Suggestion {
                insert: t.name.clone(),
                title: t.name.clone(),
                detail: tr!("query-thread-spans", count = t.spans.len().to_string()),
                dot: None,
            })
            .collect(),
        "fn" => {
            let mut names: Vec<&str> = session
                .callsites()
                .filter(|c| c.kind == sismo_format::CallsiteKind::Span && matches(&c.name))
                .map(|c| c.name.as_str())
                .collect();
            names.sort_unstable();
            names.dedup();
            names
                .into_iter()
                .map(|n| Suggestion {
                    insert: n.to_owned(),
                    title: n.to_owned(),
                    detail: String::new(),
                    dot: None,
                })
                .collect()
        }
        "level" => Level::ALL
            .iter()
            .flat_map(|l| [format!("{}+", l.as_str()), l.as_str().to_owned()])
            .filter(|v| matches(v))
            .map(|v| Suggestion {
                detail: if v.ends_with('+') {
                    tr!("query-level-or-worse")
                } else {
                    tr!("query-level-only")
                },
                dot: Level::parse(v.trim_end_matches('+')).map(theme::level),
                insert: v.clone(),
                title: v,
            })
            .collect(),
        "source" => Source::ALL
            .iter()
            .filter(|s| matches(s.as_str()))
            .map(|s| Suggestion {
                insert: s.as_str().to_owned(),
                title: s.as_str().to_owned(),
                detail: String::new(),
                dot: None,
            })
            .collect(),
        "frame" => vec![Suggestion {
            insert: "slow".into(),
            title: "slow".into(),
            detail: tr!("query-frame-slow"),
            dot: None,
        }],
        _ => Vec::new(),
    };
    out.truncate(12);
    for s in &mut out {
        s.insert = format!(
            "{key}:{}{}",
            s.insert,
            if key == "crate" || key == "thread" {
                ""
            } else {
                " "
            }
        );
    }
    out
}

/// Marks a suggestion that replaces the whole query (recent and saved ones).
const WHOLE: char = '\u{1}';

fn history(saved: &[(String, String)], recent: &[String]) -> Vec<Suggestion> {
    saved
        .iter()
        .map(|(name, query)| Suggestion {
            insert: format!("{WHOLE}{query}"),
            title: format!("★ {name}"),
            detail: query.clone(),
            dot: None,
        })
        .chain(recent.iter().take(5).map(|query| Suggestion {
            insert: format!("{WHOLE}{query}"),
            title: format!("↺ {query}"),
            detail: String::new(),
            dot: None,
        }))
        .collect()
}

fn suggestions(session: &Session, text: &str) -> Vec<Suggestion> {
    let token = last_token(text);
    let token = token.strip_prefix('-').unwrap_or(token);
    if let Some((key, value)) = token.split_once(':') {
        let prefix = value.rsplit(',').next().unwrap_or("");
        let done = value
            .rsplit_once(',')
            .map_or(String::new(), |(head, _)| format!("{head},"));
        return values(session, key, prefix)
            .into_iter()
            .map(|mut s| {
                if !done.is_empty() {
                    s.insert = s
                        .insert
                        .replacen(&format!("{key}:"), &format!("{key}:{done}"), 1);
                }
                s
            })
            .collect();
    }
    KEYS.iter()
        .filter(|(k, _, _)| token.is_empty() || k.starts_with(token))
        .map(|(k, help, example)| Suggestion {
            insert: (*k).to_owned(),
            title: (*k).to_owned(),
            detail: format!("{help} · {example}"),
            dot: None,
        })
        .collect()
}

fn apply(text: &str, insert: &str) -> String {
    if let Some(whole) = insert.strip_prefix(WHOLE) {
        return whole.to_owned();
    }
    let token = last_token(text);
    let negate = token.starts_with('-');
    let head = &text[..text.len() - token.len()];
    format!("{head}{}{insert}", if negate { "-" } else { "" })
}

#[derive(PartialEq)]
pub struct QueryBar {
    pub ws: State<Workspace>,
    pub index: usize,
}

impl Component for QueryBar {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.ws;
        let index = self.index;
        let initial = ws
            .peek()
            .sessions
            .get(index)
            .map(|v| v.query_text.clone())
            .unwrap_or_default();
        let mut text_state = use_state(move || initial);
        let mut selected = use_state(|| 0usize);
        let a11y = use_a11y();
        let focus = use_focus(a11y);
        let focused = focus.read().is_focused();

        let w = ws.read();
        let saved: Vec<(String, String)> = w
            .prefs
            .saved_queries
            .iter()
            .map(|q| (q.name.clone(), q.query.clone()))
            .collect();
        let (mut list, error) = w.sessions.get(index).map_or((Vec::new(), None), |v| {
            (
                suggestions(&v.session, &text_state.read()),
                v.query_error.clone(),
            )
        });
        if text_state.read().trim().is_empty() {
            let mut head = history(&saved, &w.prefs.recent_queries);
            head.append(&mut list);
            list = head;
        }
        drop(w);
        let current = (*selected.read()).min(list.len().saturating_sub(1));

        let mut commit = move |value: String| {
            text_state.set(value.clone());
            if let Some(view) = ws.write().sessions.get_mut(index) {
                view.set_query(value);
            }
        };
        let choices = list.clone();
        let input = Input::new(WritableSetter::new(text_state, move |v: String| {
            selected.set(0);
            commit(v);
        }))
        .a11y_id(a11y)
        .flat()
        .width(Size::px(560.))
        .placeholder(tr!("query-placeholder"))
        .on_pre_key_down(move |e: Event<KeyboardEventData>| match &e.key {
            Key::Named(NamedKey::ArrowDown) => {
                selected.set((current + 1).min(choices.len().saturating_sub(1)));
                false
            }
            Key::Named(NamedKey::ArrowUp) => {
                selected.set(current.saturating_sub(1));
                false
            }
            Key::Named(NamedKey::Enter) => {
                let value = text_state.peek().clone();
                ws.write().prefs.push_query(&value);
                true
            }
            Key::Character(c)
                if (e.modifiers.ctrl() || e.modifiers.meta()) && c.eq_ignore_ascii_case("s") =>
            {
                let value = text_state.peek().trim().to_owned();
                if !value.is_empty() {
                    let mut w = ws.write();
                    w.prefs.saved_queries.retain(|q| q.query != value);
                    w.prefs.saved_queries.insert(
                        0,
                        super::prefs::SavedQuery {
                            name: value.clone(),
                            query: value,
                        },
                    );
                    w.prefs.save();
                }
                false
            }
            Key::Named(NamedKey::Tab) => {
                if let Some(choice) = choices.get(current) {
                    let value = apply(&text_state.peek(), &choice.insert);
                    commit(value);
                }
                false
            }
            _ => true,
        });

        let popup = (focused && !list.is_empty()).then(|| {
            rect()
                .position(Position::new_absolute().top(34.).left(0.))
                .layer(Layer::Overlay)
                .width(Size::px(560.))
                .padding(6.)
                .corner_radius(8.)
                .background(theme::color(theme::BG1))
                .border(bordered(theme::LINE2))
                .shadow(
                    Shadow::new()
                        .y(8.)
                        .blur(24.)
                        .color(Color::from_argb(90, 0, 0, 0)),
                )
                .child(rect().padding((6., 6., 4., 6.)).child(caption(
                    &if last_token(&text_state.read()).contains(':') {
                        tr!("query-values")
                    } else {
                        tr!("query-filters")
                    },
                )))
                .children(list.into_iter().enumerate().map(|(i, s)| {
                    let insert = s.insert.clone();
                    rect()
                        .width(Size::fill())
                        .horizontal()
                        .content(Content::flex())
                        .cross_align(Alignment::Center)
                        .spacing(8.)
                        .padding((6., 8.))
                        .corner_radius(4.)
                        .background(theme::color(if i == current {
                            theme::BG3
                        } else {
                            theme::BG1
                        }))
                        .cursor(CursorIcon::Pointer)
                        .on_press(move |_| {
                            let value = apply(&text_state.peek(), &insert);
                            commit(value);
                            a11y.request_focus();
                        })
                        .maybe_child(s.dot.map(|d| swatch(d, 6., 1.)))
                        .child(mono(s.title, 12., theme::TEXT).width(Size::px(200.)))
                        .child(text(s.detail, 11., theme::TEXT3))
                        .into_element()
                }))
                .child(rect().padding((6., 6., 2., 6.)).child(text(
                    tr!("query-hint"),
                    11.,
                    theme::TEXT3,
                )))
        });

        rect()
            .width(Size::px(560.))
            .child(
                rect()
                    .width(Size::px(560.))
                    .height(Size::px(30.))
                    .corner_radius(4.)
                    .background(theme::color(theme::BG1))
                    .border(bordered(if error.is_some() {
                        theme::ERROR
                    } else if focused {
                        theme::TEXT2
                    } else {
                        theme::LINE2
                    }))
                    .center()
                    .font_family(theme::MONO)
                    .font_size(12.)
                    .child(input),
            )
            .maybe_child(error.filter(|_| focused).map(|e| {
                rect()
                    .position(Position::new_absolute().top(34.).left(0.))
                    .layer(Layer::Overlay)
                    .padding((4., 8.))
                    .corner_radius(4.)
                    .background(theme::color(theme::REC_DIM))
                    .child(text(e, 11., theme::ERROR))
            }))
            .maybe_child(popup)
    }
}
