//! Skia helpers for the canvas-heavy views (timeline, flamegraph, charts).

use std::cell::RefCell;

use freya_engine::prelude::{
    Canvas, Data, Font, FontMgr, Paint, PaintStyle, SkColor, SkRect, Typeface,
};

use super::theme::{self, Rgb};

thread_local! {
    static FACES: RefCell<Option<(Typeface, Typeface)>> = const { RefCell::new(None) };
}

fn faces() -> Option<(Typeface, Typeface)> {
    FACES.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let mgr = FontMgr::new();
            let mono = mgr.new_from_data(Data::new_copy(theme::MONO_REGULAR), None)?;
            let sans = mgr.new_from_data(Data::new_copy(theme::SANS_REGULAR), None)?;
            *slot = Some((mono, sans));
        }
        slot.clone()
    })
}

#[must_use]
pub fn sk(rgb: Rgb) -> SkColor {
    SkColor::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// Immediate-mode drawing on a canvas in element-local coordinates.
struct Painter<'a> {
    canvas: &'a Canvas,
    paint: Paint,
    mono: Option<Typeface>,
    sans: Option<Typeface>,
}

impl<'a> Painter<'a> {
    fn new(canvas: &'a Canvas) -> Self {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        let (mono, sans) = faces().map_or((None, None), |(m, s)| (Some(m), Some(s)));
        Self {
            canvas,
            paint,
            mono,
            sans,
        }
    }

    fn rect_alpha(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: Rgb, alpha: u8) {
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_color(sk(rgb).with_a(alpha));
        self.canvas
            .draw_rect(SkRect::from_xywh(x, y, w.max(0.5), h.max(0.5)), &self.paint);
    }

    pub fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, rgb: Rgb) {
        self.paint.set_style(PaintStyle::Stroke);
        self.paint.set_stroke_width(width);
        self.paint.set_color(sk(rgb));
        let half = width / 2.;
        self.canvas.draw_rect(
            SkRect::from_xywh(
                x + half,
                y + half,
                (w - width).max(0.5),
                (h - width).max(0.5),
            ),
            &self.paint,
        );
    }

    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, rgb: Rgb, alpha: u8) {
        self.paint.set_style(PaintStyle::Stroke);
        self.paint.set_stroke_width(width);
        self.paint.set_color(sk(rgb).with_a(alpha));
        self.canvas.draw_line(from, to, &self.paint);
    }

    fn font(&self, size: f32, mono: bool) -> Option<Font> {
        let face = if mono {
            self.mono.clone()
        } else {
            self.sans.clone()
        }?;
        Some(Font::from_typeface(face, size))
    }

    /// Draws `text` with its top-left at `(x, y)`, cut with `…` to `max_w`.
    pub fn text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        rgb: Rgb,
        mono: bool,
        max_w: f32,
    ) {
        let Some(font) = self.font(size, mono) else {
            return;
        };
        if max_w < size {
            return;
        }
        let (_, metrics) = font.metrics();
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_color(sk(rgb));
        let width = font.measure_str(text, None).0;
        let baseline = y - metrics.ascent;
        if width <= max_w {
            self.canvas
                .draw_str(text, (x, baseline), &font, &self.paint);
            return;
        }
        let ellipsis = "…";
        let budget = max_w - font.measure_str(ellipsis, None).0;
        let mut cut = String::new();
        for c in text.chars() {
            cut.push(c);
            if font.measure_str(&cut, None).0 > budget {
                cut.pop();
                break;
            }
        }
        if cut.is_empty() {
            return;
        }
        cut.push_str(ellipsis);
        self.canvas
            .draw_str(&cut, (x, baseline), &font, &self.paint);
    }
}

/// Width of `text` at `size`, without a canvas.
#[must_use]
pub fn measure(text: &str, size: f32, mono: bool) -> f32 {
    faces()
        .map(|(m, s)| if mono { m } else { s })
        .map_or(0., |face| {
            Font::from_typeface(face, size).measure_str(text, None).0
        })
}

#[derive(Debug, Clone)]
enum Cmd {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rgb: Rgb,
        alpha: u8,
    },
    Outline {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        width: f32,
        rgb: Rgb,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
        rgb: Rgb,
        alpha: u8,
    },
    Text {
        text: String,
        x: f32,
        y: f32,
        size: f32,
        rgb: Rgb,
        mono: bool,
        max_w: f32,
    },
}

/// Drawing commands recorded during render and replayed on paint, so
/// canvases never hold a borrow of the session.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    cmds: Vec<Cmd>,
}

impl Scene {
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: Rgb) {
        self.rect_alpha(x, y, w, h, rgb, 255);
    }

    pub fn rect_alpha(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: Rgb, alpha: u8) {
        self.cmds.push(Cmd::Rect {
            x,
            y,
            w,
            h,
            rgb,
            alpha,
        });
    }

    pub fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, rgb: Rgb) {
        self.cmds.push(Cmd::Outline {
            x,
            y,
            w,
            h,
            width,
            rgb,
        });
    }

    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, rgb: Rgb, alpha: u8) {
        self.cmds.push(Cmd::Line {
            from,
            to,
            width,
            rgb,
            alpha,
        });
    }

    pub fn polyline(&mut self, points: &[(f32, f32)], width: f32, rgb: Rgb) {
        for pair in points.windows(2) {
            self.line(pair[0], pair[1], width, rgb, 255);
        }
    }

    pub fn text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        rgb: Rgb,
        mono: bool,
        max_w: f32,
    ) {
        self.cmds.push(Cmd::Text {
            text: text.to_owned(),
            x,
            y,
            size,
            rgb,
            mono,
            max_w,
        });
    }

    /// Paint callback replaying the scene in element-local coordinates.
    #[must_use]
    pub fn callback(self) -> freya::prelude::RenderCallback {
        freya::prelude::RenderCallback::new(move |ctx| {
            let freya::prelude::RenderTarget::Background { canvas, .. } = &mut ctx.target else {
                return;
            };
            let mut p = Painter::new(canvas);
            for cmd in &self.cmds {
                match cmd {
                    Cmd::Rect {
                        x,
                        y,
                        w,
                        h,
                        rgb,
                        alpha,
                    } => p.rect_alpha(*x, *y, *w, *h, *rgb, *alpha),
                    Cmd::Outline {
                        x,
                        y,
                        w,
                        h,
                        width,
                        rgb,
                    } => p.outline(*x, *y, *w, *h, *width, *rgb),
                    Cmd::Line {
                        from,
                        to,
                        width,
                        rgb,
                        alpha,
                    } => p.line(*from, *to, *width, *rgb, *alpha),
                    Cmd::Text {
                        text,
                        x,
                        y,
                        size,
                        rgb,
                        mono,
                        max_w,
                    } => {
                        p.text(text, *x, *y, *size, *rgb, *mono, *max_w);
                    }
                }
            }
        })
    }
}
