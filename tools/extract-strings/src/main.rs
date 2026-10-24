//! Extrai textos traduzíveis do código Rust para um arquivo .pot.
//!
//! O xgettext não suporta Rust, e o modo C se perde em lifetimes (`'a`) e raw strings.
//! Reconhece só as formas usadas no projeto: `gettext("...")`, `N_("...")`,
//! `ngettext("...", "...", n)` e `ngettext_n("...", "...", n)`, com literais comuns, e os
//! comentários `// Translators:` na linha imediatamente acima.
//!
//! Uso: `cargo run -p extract-strings -- SAIDA.pot ARQUIVO.rs...`

use std::fmt::Write;

#[derive(Debug, PartialEq)]
struct Message {
    msgid: String,
    plural: Option<String>,
    references: Vec<String>,
    comment: Option<String>,
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Literais de string consecutivos (separados por vírgula) logo após `call`.
fn literal_args(after_call: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cursor = after_call.trim_start();
    while let Some(rest) = cursor.strip_prefix('"') {
        let bytes = rest.as_bytes();
        let mut end = 0;
        while end < bytes.len() && bytes[end] != b'"' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        if end >= bytes.len() {
            break;
        }
        args.push(unescape(&rest[..end]));
        cursor = rest[end + 1..].trim_start();
        match cursor.strip_prefix(',') {
            Some(next) => cursor = next.trim_start(),
            None => break,
        }
    }
    args
}

/// Chamadas reconhecidas em todo o arquivo, como (posição em bytes, msgid, plural). Analisa o
/// texto inteiro, não linha a linha: os argumentos de `ngettext` costumam ocupar várias linhas.
fn calls_in(source: &str) -> Vec<(usize, String, Option<String>)> {
    let mut found = Vec::new();
    for (call, plural) in [("ngettext_n(", true), ("ngettext(", true), ("gettext(", false), ("N_(", false)] {
        let mut start = 0;
        while let Some(pos) = source[start..].find(call) {
            let at = start + pos;
            start = at + call.len();
            let preceded_by_ident = source[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
            if preceded_by_ident {
                continue;
            }
            let args = literal_args(&source[at + call.len()..]);
            match (plural, args.as_slice()) {
                (true, [singular, plural, ..]) => found.push((at, singular.clone(), Some(plural.clone()))),
                (false, [msgid, ..]) => found.push((at, msgid.clone(), None)),
                _ => {}
            }
        }
    }
    found.sort_by_key(|(at, _, _)| *at);
    found
}

fn extract(files: &[(String, String)]) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::new();
    for (path, source) in files {
        let lines: Vec<&str> = source.lines().collect();
        for (at, msgid, plural) in calls_in(source) {
            let index = source[..at].matches('\n').count();
            let comment = index
                .checked_sub(1)
                .map(|i| lines[i].trim())
                .filter(|above| above.starts_with("// Translators:"))
                .map(|above| above.trim_start_matches("//").trim().to_owned());
            let reference = format!("{path}:{}", index + 1);
            match messages.iter_mut().find(|m| m.msgid == msgid && m.plural == plural) {
                Some(message) => {
                    message.references.push(reference);
                    message.comment = message.comment.take().or(comment);
                }
                None => messages.push(Message { msgid, plural, references: vec![reference], comment }),
            }
        }
    }
    messages
}

fn quote(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
    format!("\"{escaped}\"")
}

fn render(messages: &[Message]) -> String {
    let mut out = String::from(
        "msgid \"\"\nmsgstr \"\"\n\"Project-Id-Version: readrss\\n\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Content-Transfer-Encoding: 8bit\\n\"\n\n",
    );
    for m in messages {
        if let Some(comment) = &m.comment {
            let _ = writeln!(out, "#. {comment}");
        }
        let _ = writeln!(out, "#: {}", m.references.join(" "));
        let _ = writeln!(out, "msgid {}", quote(&m.msgid));
        match &m.plural {
            None => out.push_str("msgstr \"\"\n\n"),
            Some(plural) => {
                let _ = writeln!(out, "msgid_plural {}", quote(plural));
                out.push_str("msgstr[0] \"\"\nmsgstr[1] \"\"\n\n");
            }
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((output, sources)) = args.split_first() else {
        eprintln!("uso: extract-strings SAIDA.pot ARQUIVO.rs...");
        std::process::exit(2);
    };
    let files: Vec<(String, String)> = sources
        .iter()
        .map(|path| (path.clone(), std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))))
        .collect();
    let messages = extract(&files);
    std::fs::write(output, render(&messages)).unwrap_or_else(|e| panic!("{output}: {e}"));
    println!("{} textos extraídos para {output}", messages.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_calls_comments_and_plurals() {
        let source = r##"
fn f<'a>(x: &'a str) {
    let raw = r#"não é "gettext("ignorado")"#;
    // Translators: abbreviated "minutes ago"
    let a = fill(&gettext("{n} min"), &[("n", &1)]);
    let b = ngettext_n("{n} article", "{n} articles", 2);
    let c = N_("Static \"quoted\"");
    let d = my_gettext("not mine");
}
"##;
        let messages = extract(&[("src/x.rs".into(), source.into())]);
        let ids: Vec<_> = messages.iter().map(|m| (m.msgid.as_str(), m.plural.as_deref())).collect();
        assert!(ids.contains(&("{n} min", None)));
        assert!(ids.contains(&("{n} article", Some("{n} articles"))));
        assert!(ids.contains(&("Static \"quoted\"", None)));
        assert!(!ids.iter().any(|(m, _)| *m == "not mine"));
        let min = messages.iter().find(|m| m.msgid == "{n} min").unwrap();
        assert_eq!(min.comment.as_deref(), Some("Translators: abbreviated \"minutes ago\""));
        assert!(render(&messages).contains("msgid \"Static \\\"quoted\\\"\""));
    }

    #[test]
    fn plural_arguments_may_span_lines() {
        let source = "let t = ngettext(\"{n} feed imported\",\n                 \"{n} feeds imported\", n);";
        let messages = extract(&[("a.rs".into(), source.into())]);
        assert_eq!(messages[0].plural.as_deref(), Some("{n} feeds imported"));
        assert_eq!(messages[0].references, vec!["a.rs:1"]);
    }

    #[test]
    fn merges_repeated_messages() {
        let messages = extract(&[("a.rs".into(), "gettext(\"Cancel\")\ngettext(\"Cancel\")".into())]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].references, vec!["a.rs:1", "a.rs:2"]);
    }
}
