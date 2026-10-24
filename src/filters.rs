//! Filtro de artigos por palavras-chave ou expressões regulares (por feed).
//!
//! Regras: uma por linha, procuradas no título e no texto do resumo, sem diferenciar
//! maiúsculas nem acentos ("eleicao" casa com "Eleição").
//! - Palavras simples casam só com a palavra/termo inteiro ("PT" não casa com "apto").
//! - Com `use_regex`, cada linha é uma expressão regular no estilo Perl/PCRE (inclusive
//!   lookarounds), a mesma dos filtros já salvos no banco.

use fancy_regex::{Regex, RegexBuilder};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Hide,
    Only,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Hide => "hide",
            Mode::Only => "only",
        }
    }

    pub fn parse(value: &str) -> Option<Mode> {
        match value {
            "hide" => Some(Mode::Hide),
            "only" => Some(Mode::Only),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedFilter {
    pub mode: Mode,
    pub rules: Vec<String>,
    pub use_regex: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterError {
    pub line: usize,
    pub rule: String,
    pub message: String,
}

enum Pattern {
    Term(String),
    Regex(Regex),
}

pub struct CompiledFilter {
    mode: Mode,
    patterns: Vec<Pattern>,
}

impl FeedFilter {
    pub fn from_text(mode: Mode, text: &str, use_regex: bool) -> FeedFilter {
        let rules = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
        FeedFilter { mode, rules, use_regex }
    }

    pub fn rules_text(&self) -> String {
        self.rules.join("\n")
    }

    pub fn compile(&self) -> Result<CompiledFilter, FilterError> {
        let mut patterns = Vec::with_capacity(self.rules.len());
        for (index, rule) in self.rules.iter().enumerate() {
            let source = strip_accents(rule);
            if self.use_regex {
                let regex = RegexBuilder::new(&source).case_insensitive(true).build().map_err(|e| FilterError {
                    line: index + 1,
                    rule: rule.clone(),
                    message: e.to_string(),
                })?;
                patterns.push(Pattern::Regex(regex));
            } else {
                patterns.push(Pattern::Term(source.to_lowercase()));
            }
        }
        Ok(CompiledFilter { mode: self.mode, patterns })
    }
}

impl CompiledFilter {
    pub fn hides(&self, title: &str, summary_html: &str) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let text = strip_accents(&format!("{title}\n{}", html_to_text(summary_html)));
        let lower = text.to_lowercase();
        let matched = self.patterns.iter().any(|pattern| match pattern {
            Pattern::Term(term) => contains_term(&lower, term),
            Pattern::Regex(regex) => regex.is_match(&text).unwrap_or(false),
        });
        match self.mode {
            Mode::Hide => matched,
            Mode::Only => !matched,
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Ocorrência de `term` não colada a letras/dígitos: o equivalente a `(?<!\w)termo(?!\w)`,
/// que funciona também para termos com símbolos como "C++" ou "R$".
fn contains_term(text: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }
    text.match_indices(term).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + term.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

pub fn strip_accents(text: &str) -> String {
    text.nfkd().filter(|c| !is_combining_mark(*c)).collect()
}

pub fn html_to_text(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    dom_query::Document::fragment(html).root().text().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hides(mode: Mode, rules: &str, title: &str, summary: &str, regex: bool) -> bool {
        FeedFilter::from_text(mode, rules, regex).compile().unwrap().hides(title, summary)
    }

    #[test]
    fn plain_words_ignore_case_and_accents() {
        assert!(hides(Mode::Hide, "eleicao", "ELEIÇÃO municipal", "", false));
        assert!(hides(Mode::Hide, "Eleição", "eleicao municipal", "", false));
    }

    #[test]
    fn plain_words_match_whole_words_only() {
        assert!(hides(Mode::Hide, "PT", "Reunião do PT hoje", "", false));
        assert!(!hides(Mode::Hide, "PT", "Apartamento apto para venda", "", false));
        assert!(hides(Mode::Hide, "C++", "Novidades do C++ 26", "", false));
        assert!(!hides(Mode::Hide, "C++", "Biblioteca ABC++ lançada", "", false));
    }

    #[test]
    fn plain_words_are_not_regex() {
        assert!(hides(Mode::Hide, "R$ 10", "Custa R$ 10 agora", "", false));
        assert!(!hides(Mode::Hide, "a.b", "axb", "", false));
    }

    #[test]
    fn any_rule_matches() {
        let rules = "BBB\nfutebol\n";
        assert!(hides(Mode::Hide, rules, "Final do BBB", "", false));
        assert!(hides(Mode::Hide, rules, "Rodada do futebol", "", false));
        assert!(!hides(Mode::Hide, rules, "Lançamento do Ubuntu", "", false));
    }

    #[test]
    fn searches_summary_text_not_html() {
        assert!(hides(Mode::Hide, "kernel", "Notícia", "<p>Novo <b>kernel</b> lançado</p>", false));
        assert!(!hides(Mode::Hide, "href", "Notícia", r#"<a href="x">link</a>"#, false));
    }

    #[test]
    fn only_mode_hides_what_does_not_match() {
        assert!(!hides(Mode::Only, "Ubuntu|GNOME", "Ubuntu 26.10 lançado", "", true));
        assert!(hides(Mode::Only, "Ubuntu|GNOME", "Windows 12 anunciado", "", true));
    }

    #[test]
    fn empty_rules_never_hide() {
        for mode in [Mode::Hide, Mode::Only] {
            assert!(!FeedFilter { mode, rules: vec![], use_regex: false }.compile().unwrap().hides("x", ""));
        }
    }

    #[test]
    fn regex_mode_supports_lookarounds_and_classes() {
        assert!(hides(Mode::Hide, r"^v[ií]deo", "VÍDEO: algo aconteceu", "", true));
        assert!(!hides(Mode::Hide, r"^video", "Um vídeo", "", true));
        assert!(hides(Mode::Hide, r"BBB\W26", "BBB-26 estreia", "", true));
        assert!(hides(Mode::Hide, r"(?<!a)pto\b", "o pto", "", true));
    }

    #[test]
    fn invalid_regex_reports_line() {
        let error = FeedFilter::from_text(Mode::Hide, "ok\n(unclosed\n", true).compile().err().unwrap();
        assert_eq!((error.line, error.rule.as_str()), (2, "(unclosed"));
    }

    #[test]
    fn from_text_trims_and_skips_blank_lines() {
        let filter = FeedFilter::from_text(Mode::Hide, "  a  \n\n b\n   \n", false);
        assert_eq!(filter.rules, vec!["a", "b"]);
        assert_eq!(filter.rules_text(), "a\nb");
    }

    #[test]
    fn helpers() {
        assert_eq!(strip_accents("ação é ótima"), "acao e otima");
        assert_eq!(html_to_text(""), "");
        assert_eq!(html_to_text("texto puro"), "texto puro");
        assert_eq!(html_to_text("<p>a <b>b</b></p>").trim(), "a b");
    }
}
