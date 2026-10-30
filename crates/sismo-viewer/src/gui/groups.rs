//! Crate groups editor: per-app rules deciding what counts as app,
//! framework or other.

use std::collections::HashMap;

use freya::prelude::*;
use sismo_analysis::Group;
use sismo_analysis::units::Percent;

use super::Workspace;
use super::i18n::tr;
use super::theme;
use super::widgets::{Kind, bordered, button, caption, hline, kbd, mono, panel, swatch, text};

fn segmented(current: Group, on_pick: impl FnMut(Group) + Clone + 'static) -> Element {
    rect()
        .horizontal()
        .padding(2.)
        .corner_radius(4.)
        .background(theme::color(theme::BG0))
        .border(bordered(theme::LINE2))
        .children(Group::ALL.iter().map(move |g| {
            let g = *g;
            let on = g == current;
            let mut pick = on_pick.clone();
            rect()
                .padding((4., 10.))
                .corner_radius(3.)
                .background(theme::color(if on { theme::BG3 } else { theme::BG0 }))
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| pick(g))
                .child(
                    text(
                        theme::group_label(g),
                        11.,
                        if on { theme::TEXT } else { theme::TEXT2 },
                    )
                    .font_weight(if on { 600 } else { 400 }),
                )
                .into_element()
        }))
        .into_element()
}

#[derive(PartialEq)]
pub struct GroupsDialog(pub State<Workspace>);

impl Component for GroupsDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let (app, crates, initial) = {
            let w = ws.peek();
            let Some(view) = w.active() else {
                return rect().into_element();
            };
            let app = view.session.info.app.clone();
            let window = view.window();
            let mut per_crate: HashMap<String, u64> = HashMap::new();
            for thread in &view.session.threads {
                for span in thread.spans_in(window.0, window.1) {
                    *per_crate
                        .entry(view.session.crate_of(span.callsite).to_owned())
                        .or_default() += span.self_ns;
                }
            }
            let total = per_crate.values().sum::<u64>().max(1);
            let mut crates: Vec<(String, f64)> = view
                .session
                .crates()
                .into_iter()
                .map(|c| {
                    #[allow(clippy::cast_precision_loss)]
                    let share =
                        per_crate.get(&c).copied().unwrap_or(0) as f64 / total as f64 * 100.;
                    (c, share)
                })
                .collect();
            crates.sort_by(|a, b| b.1.total_cmp(&a.1));
            let initial = w.prefs.custom_rules(&app);
            (app, crates, initial)
        };
        let mut rules = use_state(move || initial);
        let pattern = use_state(String::new);
        let mut new_group = use_state(|| Group::App);

        let mut preview = sismo_analysis::Groups::default();
        preview.set_app(&app);
        for (p, g) in rules.read().iter().rev() {
            preview.rules.insert(
                0,
                sismo_analysis::Rule {
                    pattern: p.clone(),
                    group: *g,
                },
            );
        }

        let save_app = app.clone();
        let save = move |_| {
            let list = rules.peek().clone();
            let mut w = ws.write();
            w.prefs.set_groups(&save_app, &list);
            let groups = w.prefs.groups_for(&save_app);
            for view in w
                .sessions
                .iter_mut()
                .filter(|v| v.session.info.app == save_app)
            {
                view.session.groups = groups.clone();
                view.revision += 1;
            }
            w.dialog = None;
        };

        let rule_rows = rules
            .read()
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, (p, g))| {
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .content(Content::flex())
                    .cross_align(Alignment::Center)
                    .spacing(10.)
                    .padding((6., 10.))
                    .corner_radius(4.)
                    .background(theme::color(theme::BG0))
                    .border(bordered(theme::LINE))
                    .child(mono(p, 12., theme::TEXT).width(Size::flex(1.)))
                    .child(segmented(g, move |group| {
                        if let Some(rule) = rules.write().get_mut(i) {
                            rule.1 = group;
                        }
                    }))
                    .child(
                        rect()
                            .padding((4., 8.))
                            .cursor(CursorIcon::Pointer)
                            .on_press(move |_| {
                                rules.write().remove(i);
                            })
                            .child(text("×", 14., theme::TEXT3)),
                    )
                    .into_element()
            });

        let crate_rows = crates.into_iter().take(14).map(|(krate, share)| {
            let group = preview.classify(&krate);
            let assign = krate.clone();
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .spacing(10.)
                .padding((3., 0.))
                .child(swatch(theme::group(group), 8., 1.))
                .child(mono(krate, 12., theme::TEXT).width(Size::flex(1.)))
                .child(mono(Percent(share).to_string(), 12., theme::TEXT2).width(Size::px(60.)))
                .child(segmented(group, move |g| {
                    let mut list = rules.write();
                    list.retain(|(p, _)| *p != assign);
                    list.insert(0, (assign.clone(), g));
                }))
                .into_element()
        });

        panel(720.)
            .padding(24.)
            .spacing(12.)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .content(Content::flex())
                    .cross_align(Alignment::Center)
                    .child(
                        text(tr!("groups-title", app = app), 15., theme::TEXT)
                            .font_weight(600)
                            .width(Size::flex(1.)),
                    )
                    .child(kbd("Esc")),
            )
            .child(text(tr!("groups-hint"), 12., theme::TEXT2))
            .child(caption(&tr!("groups-rules")))
            .children(rule_rows)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .content(Content::flex())
                    .cross_align(Alignment::Center)
                    .spacing(10.)
                    .child(
                        rect()
                            .width(Size::flex(1.))
                            .height(Size::px(30.))
                            .corner_radius(4.)
                            .background(theme::color(theme::BG0))
                            .border(bordered(theme::LINE2))
                            .center()
                            .font_family(theme::MONO)
                            .font_size(12.)
                            .child(
                                Input::new(pattern)
                                    .flat()
                                    .width(Size::fill())
                                    .placeholder(tr!("groups-pattern-placeholder")),
                            ),
                    )
                    .child(segmented(*new_group.read(), move |g| new_group.set(g)))
                    .child(button(tr!("groups-add"), Kind::Secondary, move |_| {
                        let value = pattern.peek().trim().replace('-', "_");
                        if !value.is_empty() {
                            let g = *new_group.peek();
                            rules.write().insert(0, (value, g));
                        }
                    })),
            )
            .child(text(tr!("groups-pattern-hint"), 11., theme::TEXT3))
            .child(caption(&tr!("groups-crates")))
            .children(crate_rows)
            .child(hline())
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .main_align(Alignment::End)
                    .spacing(8.)
                    .child(button(tr!("groups-reset"), Kind::Secondary, move |_| {
                        rules.set(Vec::new());
                    }))
                    .child(button(tr!("groups-cancel"), Kind::Secondary, move |_| {
                        ws.write().dialog = None;
                    }))
                    .child(button(tr!("groups-done"), Kind::Primary, save)),
            )
            .into_element()
    }
}
