//! Grafito palette and type tokens.

use freya::prelude::Color;
use sismo_analysis::Group;
use sismo_format::Level;

use super::i18n::tr;

pub type Rgb = (u8, u8, u8);

pub const BG0: Rgb = (0x16, 0x16, 0x15);
pub const BG1: Rgb = (0x1D, 0x1D, 0x1B);
pub const BG2: Rgb = (0x26, 0x26, 0x24);
pub const BG3: Rgb = (0x31, 0x31, 0x2E);
pub const LINE: Rgb = (0x2A, 0x2A, 0x28);
pub const LINE2: Rgb = (0x3A, 0x3A, 0x37);
pub const TEXT: Rgb = (0xEC, 0xEB, 0xE6);
pub const TEXT2: Rgb = (0xA9, 0xA8, 0xA1);
pub const TEXT3: Rgb = (0x74, 0x73, 0x6D);
pub const APP: Rgb = (0xDE, 0x85, 0x52);
pub const FRAMEWORK: Rgb = (0x8F, 0xA6, 0xBF);
pub const OTHER: Rgb = (0x77, 0x75, 0x6E);
pub const ERROR: Rgb = (0xE5, 0x48, 0x4D);
pub const WARN: Rgb = (0xD6, 0xB6, 0x56);
pub const INFO: Rgb = (0x8F, 0xA6, 0xBF);
pub const DEBUG: Rgb = (0x74, 0x73, 0x6D);
pub const REC: Rgb = (0xE5, 0x48, 0x4D);
pub const REC_DIM: Rgb = (0x3A, 0x1F, 0x1F);
pub const WARN_DIM: Rgb = (0x2B, 0x26, 0x14);

pub const SANS: &str = "Plex Sans";
pub const MONO: &str = "Plex Mono";

pub const SANS_REGULAR: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf");
pub const SANS_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-SemiBold.ttf");
pub const MONO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf");
pub const MONO_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/IBMPlexMono-Medium.ttf");
pub const MONO_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/IBMPlexMono-SemiBold.ttf");

#[must_use]
pub fn color(rgb: Rgb) -> Color {
    Color::from_rgb(rgb.0, rgb.1, rgb.2)
}

#[must_use]
pub const fn group(group: Group) -> Rgb {
    match group {
        Group::App => APP,
        Group::Framework => FRAMEWORK,
        Group::Other => OTHER,
    }
}

#[must_use]
pub fn group_label(group: Group) -> String {
    match group {
        Group::App => tr!("group-app"),
        Group::Framework => tr!("group-framework"),
        Group::Other => tr!("group-other"),
    }
}

#[must_use]
pub const fn level(level: Level) -> Rgb {
    match level {
        Level::Panic | Level::Error => ERROR,
        Level::Warn => WARN,
        Level::Info => INFO,
        Level::Debug | Level::Trace => DEBUG,
    }
}
