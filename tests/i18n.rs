//! Garante que todo texto marcado para tradução no código Rust está traduzido nos catálogos.
//!
//! Falha quando alguém adiciona/altera um texto em gettext()/ngettext()/N_() e esquece de
//! rodar scripts/update-translations.sh e traduzir: o caso típico de regressão de i18n.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const LANGUAGES: [&str; 2] = ["pt_BR", "es"];

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn literals_after(source: &str, call: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(pos) = rest.find(call) {
        let before = rest[..pos].chars().next_back();
        rest = &rest[pos + call.len()..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let mut args = Vec::new();
        let mut cursor = rest.trim_start();
        while let Some(stripped) = cursor.strip_prefix('"') {
            let mut end = 0;
            let bytes = stripped.as_bytes();
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            args.push(unescape(&stripped[..end]));
            cursor = stripped[end + 1..].trim_start();
            match cursor.strip_prefix(',') {
                Some(after) => cursor = after.trim_start(),
                None => break,
            }
        }
        if !args.is_empty() {
            found.push(args);
        }
    }
    found
}

fn source_messages() -> BTreeSet<(String, Option<String>)> {
    let mut messages = BTreeSet::new();
    for entry in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src")).unwrap() {
        let source = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        for call in ["gettext(", "N_("] {
            for args in literals_after(&source, call) {
                messages.insert((args[0].clone(), None));
            }
        }
        for call in ["ngettext(", "ngettext_n("] {
            for args in literals_after(&source, call) {
                if args.len() >= 2 {
                    messages.insert((args[0].clone(), Some(args[1].clone())));
                }
            }
        }
    }
    messages
}

fn po_string(lines: &[&str]) -> String {
    lines.iter().map(|l| unescape(l.trim().trim_matches('"'))).collect()
}

/// msgid -> traduções (uma, ou as formas de plural).
fn parse_po(lang: &str) -> HashMap<String, Vec<String>> {
    let text = std::fs::read_to_string(format!("{ROOT}/po/{lang}.po")).unwrap();
    let mut catalog = HashMap::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.starts_with('#')).collect();
        let mut fields: Vec<(String, Vec<&str>)> = Vec::new();
        for line in lines {
            if line.starts_with('"') {
                if let Some(last) = fields.last_mut() {
                    last.1.push(line);
                }
            } else if let Some((key, value)) = line.split_once(' ') {
                fields.push((key.to_owned(), vec![value]));
            }
        }
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| po_string(v));
        let Some(msgid) = get("msgid").filter(|m| !m.is_empty()) else { continue };
        let translations: Vec<String> = match get("msgstr") {
            Some(single) => vec![single],
            None => (0..2).filter_map(|i| get(&format!("msgstr[{i}]"))).collect(),
        };
        catalog.insert(msgid, translations);
    }
    catalog
}

fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == '{' {
            if let Some(end) = chars[i..].iter().position(|c| *c == '}') {
                found.insert(chars[i..i + end + 1].iter().collect());
            }
        } else if *c == '%' && i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic() {
            found.insert(format!("%{}", chars[i + 1]));
        }
    }
    found
}

#[test]
fn source_scan_finds_messages() {
    let messages = source_messages();
    assert!(messages.len() > 80, "{}", messages.len());
    assert!(messages.contains(&("Unread".to_owned(), None)));
    assert!(messages.contains(&("{n} new article".to_owned(), Some("{n} new articles".to_owned()))));
}

#[test]
fn every_message_is_translated_and_keeps_placeholders() {
    for lang in LANGUAGES {
        let catalog = parse_po(lang);
        let mut missing = Vec::new();
        for (msgid, plural) in source_messages() {
            match catalog.get(&msgid) {
                Some(translations) if !translations.is_empty() && translations.iter().all(|t| !t.is_empty()) => {
                    let expected = placeholders(&msgid);
                    for translation in translations {
                        assert_eq!(placeholders(translation), expected, "{lang}: {msgid:?} -> {translation:?}");
                    }
                    if plural.is_some() {
                        assert_eq!(translations.len(), 2, "{lang}: plural de {msgid:?}");
                    }
                }
                _ => missing.push(msgid),
            }
        }
        assert!(missing.is_empty(), "sem tradução em {lang} (rode scripts/update-translations.sh e traduza po/{lang}.po): {missing:?}");
    }
}

#[test]
fn compiled_catalogs_exist() {
    for lang in LANGUAGES {
        assert!(Path::new(&format!("{ROOT}/data/locale/{lang}/LC_MESSAGES/readrss.mo")).exists());
    }
}
