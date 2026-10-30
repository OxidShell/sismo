//! Fluent translations (Spanish and English) on top of freya-i18n.

use freya::i18n::{I18n, I18nConfig, LanguageIdentifier, Locale};

use super::prefs::Language;

const EN: &str = concat!(
    include_str!("../../assets/i18n/en-US/main.ftl"),
    "\n",
    include_str!("../../assets/i18n/en-US/settings.ftl"),
);
const ES: &str = concat!(
    include_str!("../../assets/i18n/es-ES/main.ftl"),
    "\n",
    include_str!("../../assets/i18n/es-ES/settings.ftl"),
);

fn id(language: Language) -> LanguageIdentifier {
    language.tag().parse().unwrap_or_default()
}

/// Both locales, starting with `language`, English as fallback.
#[must_use]
pub fn config(language: Language) -> I18nConfig {
    I18nConfig::new(id(language))
        .with_locale(Locale::new_static(id(Language::English), EN))
        .with_locale(Locale::new_static(id(Language::Spanish), ES))
        .with_fallback(id(Language::English))
}

pub fn set_language(language: Language) {
    if let Some(mut i18n) = I18n::try_get() {
        let _ = i18n.try_set_language(id(language));
    }
}

/// Translation of `key`, or the key itself when it is missing.
#[must_use]
pub fn text(key: &str, args: Option<&freya_i18n::fluent::FluentArgs>) -> String {
    I18n::try_get()
        .and_then(|i18n| i18n.try_translate_with_args(key, args).ok())
        .unwrap_or_else(|| key.to_owned())
}

/// `tr!("key")` or `tr!("key", name = value, …)`.
macro_rules! tr {
    ($key:expr) => {
        $crate::gui::i18n::text($key, None)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = freya_i18n::fluent::FluentArgs::new();
        $(args.set(stringify!($name), $value);)+
        $crate::gui::i18n::text($key, Some(&args))
    }};
}

pub(crate) use tr;
