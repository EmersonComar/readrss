mod common;

use common::{FakeWeb, Response, TempDir, image_bytes, item, rss, simple_item};
use gtk::gdk_pixbuf::Pixbuf;
use readrss::db::now;
use readrss::net::{self, Client, NetError};

fn feed_xml(items: &str, web: &FakeWeb) -> Response {
    Response::new(200, "application/rss+xml", rss(items, "Test Feed", &web.url("/"), ""))
}

#[test]
fn discover_accepts_feed_url() {
    let web = FakeWeb::start();
    web.add("/feed.xml", feed_xml("", &web));
    assert_eq!(net::discover_feed_url(&Client::new(), &web.url("/feed.xml")), Ok(web.url("/feed.xml")));
}

#[test]
fn discover_finds_alternate_link_in_site_html() {
    let web = FakeWeb::start();
    web.add(
        "/",
        Response::new(
            200,
            "text/html",
            r#"<html><head><link rel="stylesheet" href="/s.css">
        <link rel="alternate" type="application/rss+xml" href="/feed.xml"></head><body>hi</body></html>"#,
        ),
    );
    assert_eq!(net::discover_feed_url(&Client::new(), &web.url("/")), Ok(web.url("/feed.xml")));
}

#[test]
fn discover_errors() {
    let web = FakeWeb::start();
    web.add("/", Response::new(200, "text/html", "<html><head><title>x</title></head><body>no feed</body></html>"));
    web.add("/gone", Response::new(410, "text/plain", "gone"));
    assert_eq!(net::discover_feed_url(&Client::new(), &web.url("/")), Err(NetError::NoFeedFound));
    assert_eq!(net::discover_feed_url(&Client::new(), &web.url("/gone")), Err(NetError::Status(410)));
}

#[test]
fn fetch_parses_metadata_and_entries() {
    let web = FakeWeb::start();
    let items = simple_item("a", "First", "https://site.test/a") + &simple_item("b", "Second", "https://site.test/b");
    web.add(
        "/feed.xml",
        Response::new(
            200,
            "application/rss+xml",
            rss(&items, "My Feed", "https://site.test/", "https://site.test/logo.png"),
        )
        .header("ETag", "\"v1\"")
        .header("Last-Modified", "Mon"),
    );
    let result = net::fetch_feed(&Client::new(), &web.url("/feed.xml"), None, None).unwrap();
    assert!(!result.not_modified);
    assert_eq!((result.title.as_str(), result.site_url.as_str()), ("My Feed", "https://site.test/"));
    assert_eq!((result.etag.as_deref(), result.modified.as_deref()), (Some("\"v1\""), Some("Mon")));
    assert_eq!(result.articles.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), vec!["First", "Second"]);
    assert_eq!(result.articles.iter().map(|a| a.guid.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    assert_eq!(result.icon_urls, vec!["https://site.test/logo.png"]);
}

#[test]
fn fetch_sends_conditional_headers_and_handles_304() {
    let web = FakeWeb::start();
    let body = rss("", "x", "https://site.test/", "");
    web.route("/feed.xml", move |req| {
        if req.headers.get("if-none-match").map(String::as_str) == Some("\"v1\"") {
            Response::new(304, "text/plain", "")
        } else {
            Response::new(200, "application/rss+xml", body.clone()).header("ETag", "\"v1\"")
        }
    });
    let result = net::fetch_feed(&Client::new(), &web.url("/feed.xml"), Some("\"v1\""), Some("Mon")).unwrap();
    assert!(result.not_modified);
    assert_eq!(result.etag.as_deref(), Some("\"v1\""));
    let sent = &web.requests()[0].headers;
    assert_eq!((sent["if-none-match"].as_str(), sent["if-modified-since"].as_str()), ("\"v1\"", "Mon"));
}

#[test]
fn fetch_errors() {
    let web = FakeWeb::start();
    web.add("/text", Response::new(200, "text/plain", "this is not a feed <<<"));
    web.add("/boom", Response::new(500, "text/plain", "boom"));
    assert!(matches!(net::fetch_feed(&Client::new(), &web.url("/text"), None, None), Err(NetError::InvalidFeed(_))));
    assert_eq!(net::fetch_feed(&Client::new(), &web.url("/boom"), None, None).err(), Some(NetError::Status(500)));
}

#[test]
fn entry_prefers_full_content_and_has_fallbacks() {
    let web = FakeWeb::start();
    let items = item(
        "a",
        "T",
        "https://site.test/a",
        "short",
        "<p>much longer full content</p>",
        "Mon, 19 Oct 2026 10:00:00 GMT",
    ) + &item("", "", "https://site.test/no-guid", "s", "", "")
        + &item("f", "Future", "https://site.test/f", "s", "", "Fri, 01 Jan 2100 00:00:00 GMT");
    web.add("/feed.xml", feed_xml(&items, &web));
    let before = now();
    let articles = net::fetch_feed(&Client::new(), &web.url("/feed.xml"), None, None).unwrap().articles;
    assert!(articles[0].summary.contains("much longer full content"));
    assert_eq!(articles[1].guid, "https://site.test/no-guid", "sem <guid>, usa o link (regra dos artigos já salvos)");
    assert_eq!(articles[1].title, "", "o título vazio é preenchido na thread da interface");
    assert!(articles[1].published >= before.floor());
    assert!(articles[2].published <= now());
}

#[test]
fn atom_feed_is_supported() {
    let web = FakeWeb::start();
    web.add(
        "/atom",
        Response::new(
            200,
            "application/atom+xml",
            r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom"><title>Atom Feed</title><link href="https://atom.test/"/>
        <id>urn:x</id><updated>2026-10-19T10:00:00Z</updated>
        <entry><title>Entry</title><link href="https://atom.test/1"/><id>urn:1</id>
        <updated>2026-10-19T10:00:00Z</updated><summary>s</summary></entry></feed>"#,
        ),
    );
    let result = net::fetch_feed(&Client::new(), &web.url("/atom"), None, None).unwrap();
    assert_eq!(result.title, "Atom Feed");
    assert_eq!((result.articles[0].guid.as_str(), result.articles[0].link.as_str()), ("urn:1", "https://atom.test/1"));
}

fn assert_png_64(path: Option<String>) {
    let path = path.expect("ícone resolvido");
    let pixbuf = Pixbuf::from_file(&path).unwrap();
    assert_eq!((pixbuf.width(), pixbuf.height()), (net::ICON_SIZE, net::ICON_SIZE));
}

#[test]
fn icon_prefers_largest_declared_icon() {
    let web = FakeWeb::start();
    let dir = TempDir::new();
    web.add(
        "/",
        Response::new(
            200,
            "text/html",
            r#"<html><head><link rel="icon" sizes="16x16" href="/small.png">
        <link rel="icon" sizes="192x192" href="/big.png"></head></html>"#,
        ),
    );
    web.add("/small.png", Response::new(200, "image/png", image_bytes("png", 16)));
    web.add("/big.png", Response::new(200, "image/png", image_bytes("png", 192)));
    assert_png_64(net::resolve_icon(&Client::new(), &web.url("/feed"), &web.url("/"), &[], dir.path()));
    assert!(!web.paths().contains(&"/small.png".to_owned()));
}

#[test]
fn icon_skips_broken_candidates_and_falls_back_to_favicon() {
    let web = FakeWeb::start();
    let dir = TempDir::new();
    web.add(
        "/",
        Response::new(
            200,
            "text/html",
            r#"<html><head><link rel="apple-touch-icon" href="/touch.png">
        <link rel="icon" href="/icon.png"></head></html>"#,
        ),
    );
    web.add("/touch.png", Response::new(200, "text/html", "<html>not an image</html>"));
    web.add("/icon.png", Response::new(200, "image/png", b"\x89PNG garbage".to_vec()));
    web.add("/favicon.ico", Response::new(200, "image/x-icon", image_bytes("ico", 32)));
    assert_png_64(net::resolve_icon(&Client::new(), &web.url("/feed"), &web.url("/"), &[], dir.path()));
    assert_eq!(web.paths().last().map(String::as_str), Some("/favicon.ico"));
}

#[test]
fn icon_from_feed_image_origin_fallback_and_none() {
    let web = FakeWeb::start();
    let dir = TempDir::new();
    web.add("/site/", Response::new(200, "text/html", "<html><head></head></html>"));
    web.add("/logo.png", Response::new(200, "image/png", image_bytes("png", 48)));
    assert_png_64(net::resolve_icon(
        &Client::new(),
        &web.url("/feed"),
        &web.url("/site/"),
        &[web.url("/logo.png")],
        dir.path(),
    ));

    let origin = FakeWeb::start();
    origin.add("/favicon.ico", Response::new(200, "image/x-icon", image_bytes("ico", 16)));
    assert_png_64(net::resolve_icon(&Client::new(), &origin.url("/deep/feed.xml"), "", &[], dir.path()));

    let empty = FakeWeb::start();
    assert_eq!(net::resolve_icon(&Client::new(), &empty.url("/feed"), &empty.url("/"), &[], dir.path()), None);
}

#[test]
fn icon_refresh_policy() {
    let dir = TempDir::new();
    let existing = dir.path().join("icon.png");
    std::fs::write(&existing, image_bytes("png", 8)).unwrap();
    let existing = existing.to_str().unwrap();
    let t = now();
    assert!(net::icon_needs_refresh(None, 0.0));
    assert!(!net::icon_needs_refresh(None, t - 3600.0));
    assert!(net::icon_needs_refresh(None, t - net::ICON_RETRY_FAILED - 1.0));
    assert!(!net::icon_needs_refresh(Some(existing), t - 2.0 * 86400.0));
    assert!(net::icon_needs_refresh(Some(existing), t - net::ICON_REFRESH - 1.0));
    assert!(net::icon_needs_refresh(Some("/nonexistent/gone.png"), t - 2.0 * 86400.0));
}

#[test]
fn extract_article_returns_clean_main_content() {
    let web = FakeWeb::start();
    let page = format!(
        r#"<html><head><title>Story</title></head><body><nav class="menu"><a href="/">Home</a></nav>
        <article class="post" style="color:red"><h1>The headline</h1><p class="lead">{}</p><p>{}</p></article>
        <footer>Copyright footer text</footer></body></html>"#,
        "The first paragraph explains in detail what happened in the city today. ".repeat(6),
        "A second paragraph adds more context, quotes and background information. ".repeat(6)
    );
    web.add("/story", Response::new(200, "text/html; charset=utf-8", page));
    web.add("/empty", Response::new(200, "text/html", "<html><body></body></html>"));
    let html = net::extract_article(&Client::new(), &web.url("/story")).unwrap();
    assert!(html.contains("first paragraph explains") && html.contains("second paragraph adds"));
    for unwanted in ["<html", "<body", "class=", "style="] {
        assert!(!html.contains(unwanted), "{unwanted} em {html}");
    }
    assert_eq!(net::extract_article(&Client::new(), &web.url("/empty")), Err(NetError::ExtractFailed));
}

#[test]
fn describe_errors_and_user_agent() {
    assert_eq!(NetError::Status(403).describe(), "HTTP 403 Forbidden");
    assert_eq!(NetError::Timeout.describe(), "timed out");
    assert_eq!(NetError::Connect.describe(), "connection failed");
    assert_eq!(NetError::Other("custom".into()).describe(), "custom");

    let web = FakeWeb::start();
    web.add("/feed.xml", feed_xml("", &web));
    net::fetch_feed(&Client::new(), &web.url("/feed.xml"), None, None).unwrap();
    assert!(web.requests()[0].headers["user-agent"].starts_with("Mozilla/5.0"));
}

#[test]
fn clean_extracted_strips_attributes() {
    assert_eq!(
        net::clean_extracted(r#"intro<p class="x" style="y" id="z">hi <b>bold</b></p>"#),
        "intro<p>hi <b>bold</b></p>"
    );
}

#[test]
fn guid_fallback_uses_the_alternate_link() {
    let web = FakeWeb::start();
    web.add(
        "/atom",
        Response::new(
            200,
            "application/atom+xml",
            r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom"><title>A</title><id>urn:x</id><updated>2026-10-19T10:00:00Z</updated>
        <entry><title>Podcast</title><updated>2026-10-19T10:00:00Z</updated>
          <link rel="enclosure" href="https://atom.test/audio.mp3"/>
          <link rel="alternate" href="https://atom.test/episode"/></entry></feed>"#,
        ),
    );
    let article = &net::fetch_feed(&Client::new(), &web.url("/atom"), None, None).unwrap().articles[0];
    assert_eq!(article.guid, "https://atom.test/episode");
    assert_eq!(article.link, "https://atom.test/episode");
}
