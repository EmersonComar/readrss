use chrono::{Local, TimeZone};
use crate::i18n::gettext;
use url::Url;

use crate::db::Article;

const CSS: &str = r#"
:root {
    color-scheme: light dark;
    --fg: #1e1e1e; --muted: #6b6b6b; --bg: #ffffff; --accent: #1c71d8;
    --code-bg: #f2f2f2; --border: #dedede;
}
@media (prefers-color-scheme: dark) {
    :root {
        --fg: #e8e8e8; --muted: #a0a0a0; --bg: #242424; --accent: #78aeed;
        --code-bg: #303030; --border: #3a3a3a;
    }
}
html { background: var(--bg); }
body {
    max-width: 44rem; margin: 0 auto; padding: 2rem 1.5rem 4rem;
    font: 17px/1.65 "Cantarell", "Inter", system-ui, sans-serif;
    color: var(--fg); background: var(--bg); overflow-wrap: anywhere;
}
header { border-bottom: 1px solid var(--border); margin-bottom: 1.5rem; padding-bottom: 1rem; }
h1.title { font-size: 1.7rem; line-height: 1.25; margin: 0 0 .5rem; }
h1.title a { color: inherit; text-decoration: none; }
.meta { color: var(--muted); font-size: .9rem; }
.badge {
    display: inline-block; margin-top: .6rem; padding: .1rem .55rem; border-radius: 99px;
    font-size: .75rem; color: var(--accent); border: 1px solid var(--accent);
}
a { color: var(--accent); }
img, video, iframe { max-width: 100%; height: auto; border-radius: 6px; }
figure { margin: 1.2rem 0; }
figcaption { color: var(--muted); font-size: .85rem; }
pre, code { background: var(--code-bg); border-radius: 4px; font-size: .9em; }
pre { padding: .8rem; overflow-x: auto; }
blockquote { margin: 1rem 0; padding-left: 1rem; border-left: 3px solid var(--border); color: var(--muted); }
table { border-collapse: collapse; max-width: 100%; display: block; overflow-x: auto; }
td, th { border: 1px solid var(--border); padding: .3rem .5rem; }
.empty { color: var(--muted); text-align: center; margin-top: 30vh; }
"#;

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

fn lang() -> String {
    crate::i18n::current_language().replace('_', "-")
}

pub fn format_date(timestamp: f64, format: &str) -> String {
    Local.timestamp_opt(timestamp as i64, 0).single().map(|d| d.format(format).to_string()).unwrap_or_default()
}

/// HTML do leitor com links já absolutos: deve ser carregado sem URL base, senão o WebKit
/// mantém um processo por site visitado.
pub fn render_article(article: &Article, full: bool) -> String {
    let source = if full { article.full_content.as_deref().unwrap_or("") } else { &article.summary };
    let mut body = absolutize_links(source, &article.link);
    if body.trim().is_empty() {
        body = format!("<p><em>{}</em></p>", escape(&gettext("This feed has no summary. Use “Full” to read the text.")));
    }
    let date = format_date(article.published, &gettext("%m/%d/%Y %H:%M"));
    let meta: Vec<String> = [article.feed_title.as_str(), article.author.as_str(), date.as_str()]
        .iter().filter(|p| !p.is_empty()).map(|p| escape(p)).collect();
    let badge = if full { format!(r#"<span class="badge">{}</span>"#, escape(&gettext("Full article"))) } else { String::new() };
    format!(
        r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8"><style>{CSS}</style></head><body>
<header><h1 class="title"><a href="{link}">{title}</a></h1><div class="meta">{meta}</div>{badge}</header>
<article>{body}</article></body></html>"#,
        lang = lang(), link = escape(&article.link), title = escape(&article.title), meta = meta.join(" · "),
    )
}

pub fn render_message(text: &str) -> String {
    format!(r#"<!doctype html><html lang="{}"><head><meta charset="utf-8"><style>{CSS}</style></head>
<body><p class="empty">{}</p></body></html>"#, lang(), escape(text))
}

pub fn absolutize_links(html: &str, base: &str) -> String {
    let Ok(base) = Url::parse(base) else { return html.to_owned() };
    if html.trim().is_empty() {
        return String::new();
    }
    let doc = dom_query::Document::fragment(html);
    for node in doc.select("[href], [src]").nodes() {
        for name in ["href", "src"] {
            if let Some(value) = node.attr(name) {
                if let Ok(absolute) = base.join(value.trim()) {
                    node.set_attr(name, absolute.as_str());
                }
            }
        }
    }
    for node in doc.select("[srcset]").nodes() {
        if let Some(srcset) = node.attr("srcset") {
            let fixed: Vec<String> = srcset
                .split(',')
                .filter_map(|candidate| {
                    let mut parts = candidate.split_whitespace();
                    let url = parts.next()?;
                    let absolute = base.join(url).map(|u| u.to_string()).unwrap_or_else(|_| url.to_owned());
                    Some(std::iter::once(absolute).chain(parts.map(str::to_owned)).collect::<Vec<_>>().join(" "))
                })
                .collect();
            node.set_attr("srcset", &fixed.join(", "));
        }
    }
    doc.select("html").inner_html().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolutizes_relative_links() {
        let html = absolutize_links(
            r#"intro &lt;x&gt; <p><img src="a.png" srcset="b.png 1x, /c.png 2x"><a href="../o">o</a></p>"#,
            "https://site.test/news/item",
        );
        assert!(html.contains(r#"src="https://site.test/news/a.png""#), "{html}");
        assert!(html.contains(r#"srcset="https://site.test/news/b.png 1x, https://site.test/c.png 2x""#), "{html}");
        assert!(html.contains(r#"href="https://site.test/o""#), "{html}");
        assert!(html.starts_with("intro &lt;x&gt;"), "{html}");
        assert!(!html.contains("<html"), "{html}");
    }
}
