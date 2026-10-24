//! Importação/exportação de OPML.

use std::path::Path;

use quick_xml::{Reader, XmlVersion};
use quick_xml::events::Event;

use crate::db::Feed;
use crate::html::escape;

/// (url, título) de todos os outlines com `xmlUrl`, inclusive aninhados em pastas.
pub fn parse_opml(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut reader = Reader::from_str(&text);
    let mut feeds = Vec::new();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(element) | Event::Empty(element) if element.name().as_ref() == "outline" => {
                let (mut url, mut title, mut label): (Option<String>, Option<String>, Option<String>) = (None, None, None);
                for attribute in element.attributes().flatten() {
                    let value: String = attribute.normalized_value(XmlVersion::Implicit1_0).map_err(|e| e.to_string())?.into_owned();
                    match attribute.key.as_ref() {
                        "xmlUrl" => url = Some(value.trim().to_owned()),
                        "title" => title = Some(value),
                        "text" => label = Some(value),
                        _ => {}
                    }
                }
                if let Some(url) = url.filter(|u| !u.is_empty()) {
                    let name = title.filter(|t| !t.is_empty()).or(label).unwrap_or_default();
                    feeds.push((url, name));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(feeds)
}

pub fn write_opml(path: &Path, feeds: &[Feed]) -> std::io::Result<()> {
    let mut out = String::from("<?xml version='1.0' encoding='utf-8'?>\n<opml version=\"2.0\">\n");
    out.push_str("  <head>\n    <title>readrss</title>\n  </head>\n  <body>\n");
    for feed in feeds {
        let title = escape(feed.display_title());
        out.push_str(&format!(
            "    <outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{}\" htmlUrl=\"{}\" />\n",
            escape(&feed.url),
            escape(&feed.site_url)
        ));
    }
    out.push_str("  </body>\n</opml>\n");
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tempdir::TempDir;

    const NESTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0"><head><title>export</title></head><body>
  <outline text="Notícias" title="Notícias">
    <outline type="rss" text="g1" title="g1" xmlUrl="https://g1.globo.com/rss/g1/"/>
    <outline type="rss" text="Only text" xmlUrl=" https://text.test/feed "/>
  </outline>
  <outline type="rss" title="Top level" xmlUrl="https://top.test/rss"/>
  <outline text="folder without feeds"/>
</body></opml>"#;

    fn feed(id: i64, url: &str, title: &str, site: &str) -> Feed {
        Feed {
            id,
            url: url.into(),
            title: title.into(),
            site_url: site.into(),
            icon_path: None,
            icon_checked: 0.0,
            etag: None,
            modified: None,
            last_error: None,
            custom_title: None,
            unread: 0,
            has_filter: false,
        }
    }

    #[test]
    fn reads_nested_outlines() {
        let dir = TempDir::new();
        let path = dir.path().join("in.opml");
        std::fs::write(&path, NESTED).unwrap();
        assert_eq!(
            parse_opml(&path).unwrap(),
            vec![
                ("https://g1.globo.com/rss/g1/".into(), "g1".into()),
                ("https://text.test/feed".into(), "Only text".into()),
                ("https://top.test/rss".into(), "Top level".into()),
            ]
        );
    }

    #[test]
    fn write_then_parse_roundtrip() {
        let dir = TempDir::new();
        let path = dir.path().join("out.opml");
        let feeds = [feed(1, "https://a.test/feed", "A & B <news>", "https://a.test/"), feed(2, "https://b.test/feed", "", "")];
        write_opml(&path, &feeds).unwrap();
        assert_eq!(
            parse_opml(&path).unwrap(),
            vec![
                ("https://a.test/feed".into(), "A & B <news>".into()),
                ("https://b.test/feed".into(), "https://b.test/feed".into()),
            ]
        );
    }
}
