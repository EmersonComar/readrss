//! Leitor (HTML) e tempo relativo. Um único teste: trocar de idioma altera estado global do
//! processo (LANGUAGE), e testes paralelos interfeririam entre si.

use chrono::{Local, TimeZone};
use readrss::db::{Article, now};
use readrss::html::{render_article, render_message};
use readrss::i18n;
use readrss::window::relative_time;

fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> f64 {
    Local.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap().timestamp() as f64
}

fn article() -> Article {
    Article {
        id: 1,
        feed_id: 1,
        title: "Title".into(),
        link: "https://site.test/a".into(),
        author: "Ann".into(),
        summary: "<p>summary text</p>".into(),
        full_content: None,
        published: local(2026, 10, 20, 14, 30),
        is_read: false,
        is_starred: false,
        feed_title: "Feed".into(),
    }
}

#[test]
fn reader_and_relative_time_in_each_language() {
    i18n::init("en");

    let html = render_article(&article(), false);
    assert!(html.contains("<p>summary text</p>"));
    assert!(html.contains("Feed · Ann · 10/20/2026 14:30"), "{html}");
    assert!(!html.contains("Full article"));
    assert!(html.contains(r#"<html lang="en">"#));

    let full = Article { full_content: Some("<p>the whole story</p>".into()), ..article() };
    let html = render_article(&full, true);
    assert!(html.contains("<p>the whole story</p>") && !html.contains("summary text") && html.contains("Full article"));

    let hostile = Article {
        title: "<script>alert(1)</script>".into(),
        link: r#"https://x.test/"onmouseover="x"#.into(),
        ..article()
    };
    let html = render_article(&hostile, false);
    assert!(!html.contains("<script>alert(1)</script>") && html.contains("&lt;script&gt;"));
    assert!(!html.contains(r#""onmouseover=""#));

    let empty = Article { summary: String::new(), ..article() };
    assert!(render_article(&empty, false).contains("Use “Full” to read the text."));
    assert!(render_message("<b>error</b>").contains("&lt;b&gt;error&lt;/b&gt;"));

    let t = now();
    let old = local(2025, 1, 31, 12, 0);
    assert_eq!(relative_time(t - 10.0), "now");
    assert_eq!(relative_time(t - 5.0 * 60.0), "5 min");
    assert_eq!(relative_time(t - 3.0 * 3600.0), "3 h");
    assert_eq!(relative_time(t - 2.0 * 86400.0), "2 d");
    assert_eq!(relative_time(old), "01/31/25");

    i18n::set_language("pt_BR");
    let html = render_article(&full, true);
    assert!(html.contains(r#"<html lang="pt-BR">"#), "{html}");
    assert!(html.contains("20/10/2026 14:30") && html.contains("Artigo completo"));

    i18n::set_language("es");
    assert_eq!(relative_time(t - 10.0), "ahora");
    assert_eq!(relative_time(old), "31/01/25");

    i18n::set_language(i18n::SYSTEM);
}
