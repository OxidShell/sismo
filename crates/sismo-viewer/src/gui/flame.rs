//! Flamegraph tab: aggregated icicle over the window, app/framework split
//! and the callers/callees of the selected function.

use std::cell::RefCell;
use std::collections::HashMap;

use freya::prelude::*;
use sismo_analysis::units::{Nanos, Percent};
use sismo_analysis::{FlameNode, Group, Session};

use super::Workspace;
use super::draw::Scene;
use super::theme;
use super::timeline::scene;
use super::widgets::{Kind, button, caption, hline, mono, swatch, text, vline};
use crate::workspace::{SessionView, Tab};

const ROW_H: f32 = 22.;
const PAD: f32 = 16.;

type Hit = (f32, f32, f32, Vec<u32>);

thread_local! {
    /// Last drawn node rectangles per session (`x, y, w, path`), for clicks.
    static HITS: RefCell<HashMap<u64, Vec<Hit>>> = RefCell::new(HashMap::new());
}

fn focused<'a>(root: &'a FlameNode, path: &[u32]) -> (&'a FlameNode, usize) {
    let mut node = root;
    let mut depth = 0;
    for callsite in path {
        match node.children.iter().find(|c| c.callsite == *callsite) {
            Some(child) => {
                node = child;
                depth += 1;
            }
            None => break,
        }
    }
    (node, depth)
}

struct Icicle<'a> {
    session: &'a Session,
    root: &'a FlameNode,
    focus: &'a [u32],
    selected: Option<u32>,
}

impl Icicle<'_> {
    fn draw(&self, p: &mut Scene, w: f32, h: f32) -> Vec<Hit> {
        p.rect(0., 0., w, h, theme::BG0);
        let (start, depth) = focused(self.root, self.focus);
        let total = start.total_ns.max(1);
        let width = w - 2. * PAD;
        let mut hits = Vec::new();
        let mut node = self.root;
        for (row, callsite) in self.focus.iter().take(depth).enumerate() {
            let Some(child) = node.children.iter().find(|c| c.callsite == *callsite) else {
                break;
            };
            #[allow(clippy::cast_precision_loss)]
            let y = 12. + row as f32 * ROW_H;
            p.rect_alpha(
                PAD,
                y,
                width - 1.,
                ROW_H - 2.,
                theme::group(self.session.group_of(child.callsite)),
                90,
            );
            p.text(
                self.session.name(child.callsite),
                PAD + 6.,
                y + 4.,
                11.,
                theme::TEXT,
                true,
                width - 12.,
            );
            hits.push((PAD, y, width, self.focus[..=row].to_vec()));
            node = child;
        }
        #[allow(clippy::cast_precision_loss)]
        let top = 12. + depth as f32 * ROW_H;
        let mut stack: Vec<(&FlameNode, f32, f32, usize, Vec<u32>)> = Vec::new();
        let mut x = PAD;
        for child in &start.children {
            #[allow(clippy::cast_precision_loss)]
            let cw = child.total_ns as f32 / total as f32 * width;
            let mut path = self.focus[..depth].to_vec();
            path.push(child.callsite);
            stack.push((child, x, cw, 0, path));
            x += cw;
        }
        while let Some((node, x, cw, level, path)) = stack.pop() {
            if cw < 0.5 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let y = top + level as f32 * ROW_H;
            p.rect(
                x,
                y,
                (cw - 1.).max(0.5),
                ROW_H - 2.,
                theme::group(self.session.group_of(node.callsite)),
            );
            if self.selected == Some(node.callsite) {
                p.outline(x, y, cw.max(2.), ROW_H - 2., 2., theme::TEXT);
            }
            if cw > 50. {
                #[allow(clippy::cast_precision_loss)]
                let share = node.total_ns as f64 / total as f64 * 100.;
                let label = format!("{}  {}", self.session.name(node.callsite), Percent(share));
                p.text(&label, x + 6., y + 4., 11., theme::BG0, true, cw - 12.);
            }
            let mut cx = x;
            for child in &node.children {
                #[allow(clippy::cast_precision_loss)]
                let w = child.total_ns as f32 / node.total_ns.max(1) as f32 * cw;
                let mut child_path = path.clone();
                child_path.push(child.callsite);
                stack.push((child, cx, w, level + 1, child_path));
                cx += w;
            }
            hits.push((x, y, cw, path));
        }
        if start.children.is_empty() {
            p.text(
                "Nada que mostrar en este rango con esta consulta",
                PAD,
                top + 4.,
                12.,
                theme::TEXT3,
                false,
                width,
            );
        }
        hits
    }
}

fn depth_of(node: &FlameNode) -> usize {
    1 + node.children.iter().map(depth_of).max().unwrap_or(0)
}

struct Sandwich {
    callers: Vec<(u32, u64)>,
    callees: Vec<(u32, u64)>,
    total: u64,
    own: u64,
}

impl Sandwich {
    fn of(root: &FlameNode, callsite: u32) -> Self {
        let mut callers: HashMap<u32, u64> = HashMap::new();
        let mut callees: HashMap<u32, u64> = HashMap::new();
        let (mut total, mut own) = (0, 0);
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            for child in &node.children {
                if child.callsite == callsite {
                    *callers.entry(node.callsite).or_default() += child.total_ns;
                    total += child.total_ns;
                    own += child.self_ns;
                    for grandchild in &child.children {
                        *callees.entry(grandchild.callsite).or_default() += grandchild.total_ns;
                    }
                }
                stack.push(child);
            }
        }
        let sorted = |map: HashMap<u32, u64>| {
            let mut list: Vec<(u32, u64)> = map.into_iter().collect();
            list.sort_by_key(|e| std::cmp::Reverse(e.1));
            list.truncate(8);
            list
        };
        Self {
            callers: sorted(callers),
            callees: sorted(callees),
            total,
            own,
        }
    }
}

fn share_row(session: &Session, callsite: u32, label: Option<&str>, ns: u64, max: u64) -> Element {
    let name = label.map_or_else(
        || {
            if callsite == 0 {
                "(raíz)".to_owned()
            } else {
                session.name(callsite).to_owned()
            }
        },
        ToOwned::to_owned,
    );
    #[allow(clippy::cast_precision_loss)]
    let share = ns as f32 / max.max(1) as f32;
    let rgb = if label.is_some() {
        theme::TEXT3
    } else {
        theme::group(session.group_of(callsite))
    };
    rect()
        .width(Size::fill())
        .spacing(4.)
        .padding((4., 0.))
        .child(
            rect()
                .horizontal()
                .content(Content::flex())
                .width(Size::fill())
                .spacing(8.)
                .cross_align(Alignment::Center)
                .child(swatch(rgb, 6., 1.))
                .child(mono(name, 12., theme::TEXT).width(Size::flex(1.)))
                .child(mono(Nanos(ns).to_string(), 12., theme::TEXT2)),
        )
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(2.))
                .background(theme::color(theme::BG3))
                .child(
                    rect()
                        .width(Size::percent(share * 100.))
                        .height(Size::px(2.))
                        .background(theme::color(rgb)),
                ),
        )
        .into_element()
}

fn side(mut ws: State<Workspace>, session: &Session, root: &FlameNode, callsite: u32) -> Rect {
    let s = Sandwich::of(root, callsite);
    let max_callers = s.callers.first().map_or(1, |c| c.1);
    let max_callees = s.callees.first().map_or(1, |c| c.1).max(s.own);
    rect()
        .width(Size::px(380.))
        .height(Size::fill())
        .padding(16.)
        .spacing(10.)
        .background(theme::color(theme::BG1))
        .child(
            rect()
                .horizontal()
                .spacing(8.)
                .cross_align(Alignment::Center)
                .child(swatch(theme::group(session.group_of(callsite)), 10., 2.))
                .child(mono(session.name(callsite), 13., theme::TEXT).font_weight(600)),
        )
        .child(mono(
            format!("total {} · propio {}", Nanos(s.total), Nanos(s.own)),
            11.,
            theme::TEXT2,
        ))
        .child(caption("Llamado por"))
        .children(
            s.callers
                .iter()
                .map(|(c, ns)| share_row(session, *c, None, *ns, max_callers)),
        )
        .child(caption("Llama a"))
        .children(
            s.callees
                .iter()
                .map(|(c, ns)| share_row(session, *c, None, *ns, max_callees)),
        )
        .child(share_row(
            session,
            callsite,
            Some("(propio)"),
            s.own,
            max_callees,
        ))
        .child(button("Ver en Funciones", Kind::Secondary, move |_| {
            if let Some(v) = ws.write().active_mut() {
                v.tab = Tab::Functions;
            }
        }))
}

fn split_bar(split: sismo_analysis::GroupSplit) -> Rect {
    rect()
        .width(Size::fill())
        .padding((12., 16.))
        .spacing(8.)
        .child(text("¿Dónde se va el tiempo?", 13., theme::TEXT).font_weight(600))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(14.))
                .horizontal()
                .corner_radius(2.)
                .children(Group::ALL.iter().map(|g| {
                    #[allow(clippy::cast_possible_truncation)]
                    let pct = split.percent(*g) as f32;
                    rect()
                        .width(Size::percent(pct))
                        .height(Size::fill())
                        .background(theme::color(theme::group(*g)))
                        .into_element()
                })),
        )
        .child(
            rect()
                .horizontal()
                .spacing(24.)
                .children(Group::ALL.iter().map(|g| {
                    mono(
                        format!(
                            "{} {} · {}",
                            theme::group_label(*g),
                            Percent(split.percent(*g)),
                            Nanos(split.get(*g))
                        ),
                        12.,
                        theme::TEXT2,
                    )
                    .into_element()
                })),
        )
}

pub fn view(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    let session = &view.session;
    let query = view.effective_query();
    let window = view.window();
    let root = session.flame(&query, None, window);
    let split = session.group_split(&query, None, window);
    let width = view.canvas.get("flame").map_or(900., |s| s.0);
    #[allow(clippy::cast_precision_loss)]
    let height = (depth_of(&root) as f32 * ROW_H + 40.).max(200.);
    let id = view.id;
    let icicle = Icicle {
        session,
        root: &root,
        focus: &view.flame_focus,
        selected: view.selected_function,
    };
    let paint = scene(width, height, |p, w, h| {
        let hits = icicle.draw(p, w, h);
        HITS.with(|cell| cell.borrow_mut().insert(id, hits));
    });
    let window_label = format!(
        "{} · {}",
        if view.selection.is_some() {
            "selección"
        } else {
            "toda la sesión"
        },
        Nanos(window.1.saturating_sub(window.0))
    );

    let canvas = rect()
        .width(Size::fill())
        .height(Size::px(height))
        .on_sized(move |e: Event<SizedEventData>| {
            let size = (e.area.width(), e.area.height());
            if let Some(v) = ws.write().active_mut() {
                v.canvas.insert("flame", size);
            }
        })
        .background(paint)
        .on_pointer_press(move |e: Event<PointerEventData>| {
            let loc = e.element_location();
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = (loc.x as f32, loc.y as f32);
            let hit = HITS.with(|h| {
                h.borrow().get(&id).and_then(|list| {
                    list.iter()
                        .find(|(hx, hy, hw, _)| {
                            x >= *hx && x < hx + hw && y >= *hy && y < hy + ROW_H
                        })
                        .map(|(_, _, _, path)| path.clone())
                })
            });
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            match hit {
                Some(path) => {
                    v.selected_function = path.last().copied();
                    v.flame_focus = path;
                }
                None => v.flame_focus.clear(),
            }
            v.revision += 1;
        });

    let side = view
        .selected_function
        .map(|callsite| side(ws, session, &root, callsite));

    rect()
        .expanded()
        .content(Content::flex())
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .padding((10., 16.))
                .spacing(16.)
                .child(text("Icicle", 13., theme::TEXT).font_weight(600))
                .child(
                    mono(
                        format!(
                            "agregado · {window_label} · {} hilos",
                            session.threads.len()
                        ),
                        12.,
                        theme::TEXT2,
                    )
                    .width(Size::flex(1.)),
                )
                .child(text(
                    "clic = enfocar · Esc = volver a la raíz",
                    11.,
                    theme::TEXT3,
                )),
        )
        .child(hline())
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.))
                .horizontal()
                .content(Content::flex())
                .child(
                    rect()
                        .width(Size::flex(1.))
                        .height(Size::fill())
                        .content(Content::flex())
                        .child(ScrollView::new().height(Size::flex(1.)).child(canvas))
                        .child(hline())
                        .child(split_bar(split)),
                )
                .maybe(side.is_some(), |r| r.child(vline()))
                .maybe_child(side),
        )
}
