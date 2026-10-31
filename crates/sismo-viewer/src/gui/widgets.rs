//! Small styled primitives shared by every screen.

use freya::prelude::*;

use super::theme::{self, Rgb};

/// Button flavours from the design system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
}

/// Text in the UI face.
pub fn text(value: impl Into<String>, size: f32, rgb: Rgb) -> Label {
    label()
        .text(value.into())
        .font_family(theme::SANS)
        .font_size(size)
        .color(theme::color(rgb))
        .max_lines(1)
        .text_overflow(TextOverflow::Ellipsis)
}

/// Text in the data (mono) face.
pub fn mono(value: impl Into<String>, size: f32, rgb: Rgb) -> Label {
    label()
        .text(value.into())
        .font_family(theme::MONO)
        .font_size(size)
        .color(theme::color(rgb))
        .max_lines(1)
        .text_overflow(TextOverflow::Ellipsis)
}

/// Uppercase section caption.
pub fn caption(value: &str) -> Label {
    text(value.to_uppercase(), 10., theme::TEXT3).font_weight(600)
}

fn border(rgb: Rgb) -> Border {
    Border::new()
        .fill(theme::color(rgb))
        .width(1.0)
        .alignment(BorderAlignment::Inner)
}

pub fn button(
    label_text: impl Into<String>,
    kind: Kind,
    on_press: impl FnMut(Event<PressEventData>) + 'static,
) -> Rect {
    let (bg, fg, stroke) = match kind {
        Kind::Primary => (theme::TEXT, theme::BG0, None),
        Kind::Secondary => (theme::BG2, theme::TEXT, Some(theme::LINE2)),
        Kind::Danger => (theme::REC, (255, 255, 255), None),
    };
    rect()
        .padding((6., 14.))
        .corner_radius(4.)
        .background(theme::color(bg))
        .map(stroke, |r, s| r.border(border(s)))
        .cursor(CursorIcon::Pointer)
        .on_press(on_press)
        .child(text(label_text, 12., fg).font_weight(500))
}

/// Compact toolbar button; `on` marks a mode that is active.
pub fn tool(
    label_text: impl Into<String>,
    on: bool,
    on_press: impl FnMut(Event<PressEventData>) + 'static,
) -> Rect {
    rect()
        .padding((3., 8.))
        .corner_radius(4.)
        .background(theme::color(if on { theme::BG3 } else { theme::BG1 }))
        .border(border(if on { theme::TEXT3 } else { theme::LINE2 }))
        .cursor(CursorIcon::Pointer)
        .on_press(on_press)
        .child(text(label_text, 11., if on { theme::TEXT } else { theme::TEXT2 }).font_weight(500))
}

/// Pill with a colored dot, used for levels, sources and states.
pub fn chip(label_text: impl Into<String>, dot: Rgb, on: bool) -> Rect {
    rect()
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .padding((3., 9.))
        .corner_radius(11.)
        .background(theme::color(if on { theme::BG3 } else { theme::BG1 }))
        .border(border(if on { dot } else { theme::LINE2 }))
        .cursor(CursorIcon::Pointer)
        .child(swatch(dot, 6., 3.))
        .child(text(label_text, 11., if on { theme::TEXT } else { theme::TEXT2 }).font_weight(500))
}

/// Filled status badge (live, REC, lost…).
pub fn badge(label_text: impl Into<String>, fg: Rgb, bg: Rgb) -> Rect {
    rect()
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .padding((3., 9.))
        .corner_radius(11.)
        .background(theme::color(bg))
        .child(swatch(fg, 6., 3.))
        .child(mono(label_text, 11., fg).font_weight(600))
}

pub fn swatch(rgb: Rgb, size: f32, radius: f32) -> Rect {
    rect()
        .width(Size::px(size))
        .height(Size::px(size))
        .corner_radius(radius)
        .background(theme::color(rgb))
}

/// Keyboard key hint.
pub fn kbd(keys: &str) -> Rect {
    rect()
        .padding((1., 5.))
        .corner_radius(3.)
        .background(theme::color(theme::BG2))
        .border(border(theme::LINE2))
        .child(mono(keys, 10., theme::TEXT2))
}

/// Thin separator line.
pub fn hline() -> Rect {
    rect()
        .width(Size::fill())
        .height(Size::px(1.))
        .background(theme::color(theme::LINE))
}

pub fn vline() -> Rect {
    rect()
        .width(Size::px(1.))
        .height(Size::fill())
        .background(theme::color(theme::LINE))
}

/// Raised panel used by popups and dialogs.
pub fn panel(width: f32) -> Rect {
    rect()
        .width(Size::px(width))
        .corner_radius(8.)
        .background(theme::color(theme::BG1))
        .border(border(theme::LINE2))
        .shadow(
            Shadow::new()
                .y(8.)
                .blur(24.)
                .color(Color::from_argb(90, 0, 0, 0)),
        )
}

pub fn bordered(rgb: Rgb) -> Border {
    border(rgb)
}

/// Full-window scrim behind a modal, closing it when clicked.
pub fn scrim(
    on_close: impl FnMut(Event<PressEventData>) + 'static,
    content: impl IntoElement,
) -> Rect {
    rect()
        .position(Position::new_global().top(0.).left(0.))
        .width(Size::window_percent(100.))
        .height(Size::window_percent(100.))
        .layer(Layer::OverlayLevel(layers::DIALOG))
        .background(Color::from_argb(140, 0, 0, 0))
        .center()
        .on_press(on_close)
        .child(
            rect()
                .on_press(|e: Event<PressEventData>| e.stop_propagation())
                .child(content),
        )
}

/// Overlay levels, bottom to top. Siblings on one level paint in no set
/// order, so everything that can be open at once gets its own.
pub mod layers {
    pub const SETTINGS: u8 = 1;
    pub const DIALOG: u8 = 2;
    pub const MENU: u8 = 3;
    pub const TOAST: u8 = 4;
}

/// Strips the frame Freya's input paints on its own, so the field around
/// it is the only box the user sees; otherwise focus draws a second
/// border and fill inside the first one.
pub trait InputExt {
    fn chromeless(self) -> Self;
}

impl InputExt for Input {
    fn chromeless(self) -> Self {
        self.flat().theme_colors(InputColorsThemePartial {
            focus_background: Some(Color::TRANSPARENT.into()),
            focus_border_fill: Some(Color::TRANSPARENT.into()),
            ..Default::default()
        })
    }
}

/// Single-line text field: one frame whose stroke follows focus, with a
/// chromeless input inside. Mono 12px, framed, 30px tall by default.
#[derive(PartialEq)]
pub struct Field {
    value: State<String>,
    placeholder: Option<String>,
    width: Size,
    height: f32,
    font_size: f32,
    radius: f32,
    sans: bool,
    framed: bool,
    auto_focus: bool,
    on_submit: Option<EventHandler<String>>,
}

impl Field {
    #[must_use]
    pub fn new(value: State<String>) -> Self {
        Self {
            value,
            placeholder: None,
            width: Size::fill(),
            height: 30.,
            font_size: 12.,
            radius: 4.,
            sans: false,
            framed: true,
            auto_focus: false,
            on_submit: None,
        }
    }

    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    #[must_use]
    pub fn width(mut self, width: impl Into<Size>) -> Self {
        self.width = width.into();
        self
    }

    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    #[must_use]
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    #[must_use]
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// UI face instead of the data (mono) face.
    #[must_use]
    pub fn sans(mut self) -> Self {
        self.sans = true;
        self
    }

    /// No background or border: the surrounding panel is the frame.
    #[must_use]
    pub fn unframed(mut self) -> Self {
        self.framed = false;
        self
    }

    #[must_use]
    pub fn auto_focus(mut self) -> Self {
        self.auto_focus = true;
        self
    }

    #[must_use]
    pub fn on_submit(mut self, on_submit: impl Into<EventHandler<String>>) -> Self {
        self.on_submit = Some(on_submit.into());
        self
    }
}

impl Component for Field {
    fn render(&self) -> impl IntoElement {
        let a11y = use_a11y();
        let focus = use_focus(a11y);
        let stroke = if focus().is_focused() {
            theme::TEXT2
        } else {
            theme::LINE2
        };
        let input = Input::new(self.value)
            .a11y_id(a11y)
            .chromeless()
            .auto_focus(self.auto_focus)
            .width(Size::fill())
            .map(self.placeholder.clone(), Input::placeholder)
            .map(self.on_submit.clone(), Input::on_submit);
        rect()
            .width(self.width.clone())
            .height(Size::px(self.height))
            .corner_radius(self.radius)
            .maybe(self.framed, |r| {
                r.background(theme::color(theme::BG0))
                    .border(border(stroke))
            })
            .center()
            .font_family(if self.sans { theme::SANS } else { theme::MONO })
            .font_size(self.font_size)
            .child(input)
    }
}
