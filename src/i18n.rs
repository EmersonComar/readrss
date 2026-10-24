use std::fmt::Display;
use std::sync::OnceLock;

use gettextrs::{LocaleCategory, bind_textdomain_codeset, bindtextdomain, textdomain};
pub use gettextrs::{gettext, ngettext};

use crate::constants::{DOMAIN, locale_dir};

pub const SYSTEM: &str = "system";
pub const LANGUAGES: &[(&str, &str)] = &[("pt_BR", "Português (Brasil)"), ("en", "English"), ("es", "Español")];

static ORIGINAL_LANGUAGE: OnceLock<Option<String>> = OnceLock::new();

/// Marca um texto para o xgettext sem traduzir agora (dados estáticos).
#[allow(non_snake_case)]
pub const fn N_(message: &'static str) -> &'static str {
    message
}

pub fn init(language: &str) {
    ORIGINAL_LANGUAGE.get_or_init(|| std::env::var("LANGUAGE").ok());
    let _ = bindtextdomain(DOMAIN, locale_dir());
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
    set_language(language);
}

/// Troca o idioma dos próximos textos. Só deve ser chamada da thread da interface: altera a
/// variável LANGUAGE, que a gettext (e o GTK) consultam a cada tradução.
pub fn set_language(code: &str) {
    let original = ORIGINAL_LANGUAGE.get_or_init(|| std::env::var("LANGUAGE").ok());
    unsafe {
        if LANGUAGES.iter().any(|(c, _)| *c == code) {
            std::env::set_var("LANGUAGE", code);
        } else if let Some(value) = original {
            std::env::set_var("LANGUAGE", value);
        } else {
            std::env::remove_var("LANGUAGE");
        }
        gettextrs::setlocale(LocaleCategory::LcAll, "");
    }
}

/// Código do catálogo efetivamente em uso (ex.: "pt_BR"), para o atributo lang do HTML.
pub fn current_language() -> String {
    let mut candidates = Vec::new();
    if let Ok(list) = std::env::var("LANGUAGE") {
        candidates.extend(list.split(':').map(str::to_owned));
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            candidates.push(value);
        }
    }
    for candidate in candidates {
        let code = candidate.split(['.', '@']).next().unwrap_or_default();
        if code.is_empty() || code == "C" || code == "POSIX" {
            continue;
        }
        if code.starts_with("en") {
            return "en".into();
        }
        for option in [code, code.split('_').next().unwrap_or(code)] {
            if locale_dir().join(option).join("LC_MESSAGES").join(format!("{DOMAIN}.mo")).exists() {
                return option.to_owned();
            }
        }
    }
    "en".into()
}

/// Substitui `{nome}` pelos valores dados: os textos traduzidos usam placeholders nomeados.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    args.iter()
        .fold(template.to_owned(), |text, (name, value)| text.replace(&format!("{{{name}}}"), &value.to_string()))
}

pub fn ngettext_n(singular: &str, plural: &str, n: usize) -> String {
    fill(&ngettext(singular, plural, n as u32), &[("n", &n)])
}
