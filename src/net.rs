//! Rede: download e parsing de feeds, descoberta, ícones e extração de artigos.
//!
//! Tudo aqui é bloqueante e roda em threads de trabalho. Nenhuma função traduz textos:
//! os erros são devolvidos como `NetError` e descritos na thread da interface.

use std::path::Path;
use std::time::Duration;

use gtk::gdk_pixbuf::{InterpType, Pixbuf, PixbufLoader};
use gtk::glib;
use gtk::prelude::*;
use ureq::tls::{TlsConfig, TlsProvider};
use ureq::{Agent, ResponseExt};
use url::Url;

use crate::db::{NewArticle, now};
use crate::i18n::{fill, gettext};

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0";
const FEED_TYPES: &[&str] =
    &["application/rss+xml", "application/atom+xml", "application/feed+json", "application/xml", "text/xml"];
const MAX_BODY: u64 = 20 * 1024 * 1024;
pub const ICON_SIZE: i32 = 64;
pub const ICON_RETRY_FAILED: f64 = 24.0 * 3600.0;
pub const ICON_REFRESH: f64 = 7.0 * 24.0 * 3600.0;

#[derive(Clone, Debug, PartialEq)]
pub enum NetError {
    Status(u16),
    Timeout,
    Connect,
    NoFeedFound,
    InvalidFeed(String),
    ExtractFailed,
    Other(String),
}

impl NetError {
    /// Mensagem curta para a interface. Deve ser chamada na thread da interface.
    pub fn describe(&self) -> String {
        match self {
            NetError::Status(code) => {
                let reason = ureq::http::StatusCode::from_u16(*code).ok().and_then(|s| s.canonical_reason());
                format!("HTTP {code} {}", reason.unwrap_or_default()).trim_end().to_owned()
            }
            NetError::Timeout => gettext("timed out"),
            NetError::Connect => gettext("connection failed"),
            NetError::NoFeedFound => gettext("No RSS/Atom feed found on this page"),
            NetError::InvalidFeed(error) => fill(&gettext("Invalid feed: {error}"), &[("error", error)]),
            NetError::ExtractFailed => gettext("Could not extract the content of this page"),
            NetError::Other(message) => message.clone(),
        }
    }
}

impl From<ureq::Error> for NetError {
    fn from(error: ureq::Error) -> Self {
        match error {
            ureq::Error::StatusCode(code) => NetError::Status(code),
            ureq::Error::Timeout(_) => NetError::Timeout,
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => NetError::Connect,
            ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => NetError::Connect,
            ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::TimedOut => NetError::Timeout,
            other => NetError::Other(other.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct Client {
    agent: Agent,
}

struct Page {
    url: String,
    content_type: String,
    etag: Option<String>,
    modified: Option<String>,
    body: Vec<u8>,
}

impl Page {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

impl Client {
    pub fn new() -> Client {
        let agent: Agent = Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_global(Some(Duration::from_secs(20)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build())
            .build()
            .into();
        Client { agent }
    }

    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<(u16, Page), NetError> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request.call()?;
        let status = response.status().as_u16();
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let content_type = header("content-type").unwrap_or_default();
        let (etag, modified) = (header("etag"), header("last-modified"));
        let final_url = response.get_uri().to_string();
        let body = if status == 304 {
            Vec::new()
        } else {
            response.body_mut().with_config().limit(MAX_BODY).read_to_vec()?
        };
        Ok((status, Page { url: final_url, content_type, etag, modified, body }))
    }

    fn get_ok(&self, url: &str) -> Result<Page, NetError> {
        match self.get(url, &[])? {
            (status, _) if status >= 400 => Err(NetError::Status(status)),
            (_, page) => Ok(page),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FetchResult {
    pub not_modified: bool,
    pub title: String,
    pub site_url: String,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub articles: Vec<NewArticle>,
    pub icon_urls: Vec<String>,
}

fn parse_feed(body: &[u8]) -> Result<feed_rs::model::Feed, String> {
    feed_rs::parser::Builder::new()
        .id_generator(|links, title, _uri| {
            preferred_link(links).or_else(|| title.as_ref().map(|t| t.content.clone())).unwrap_or_default()
        })
        .build()
        .parse(body)
        .map_err(|e| e.to_string())
}

/// Link principal (o "alternate"). Também é o GUID dos itens sem `<guid>`, a mesma regra
/// usada pelos artigos já salvos no banco: mudá-la duplicaria artigos.
fn preferred_link(links: &[feed_rs::model::Link]) -> Option<String> {
    links
        .iter()
        .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .or(links.first())
        .map(|l| l.href.clone())
}

/// Aceita a URL de um feed ou de um site; no segundo caso procura o `<link rel=alternate>`.
pub fn discover_feed_url(client: &Client, url: &str) -> Result<String, NetError> {
    let url = if url.contains("://") { url.to_owned() } else { format!("https://{url}") };
    let page = client.get_ok(&url)?;
    if parse_feed(&page.body).is_ok() {
        return Ok(page.url);
    }
    let base = Url::parse(&page.url).map_err(|e| NetError::Other(e.to_string()))?;
    let doc = dom_query::Document::from(page.text());
    for link in doc.select("link[rel][href]").nodes() {
        let rel = link.attr("rel").unwrap_or_default().to_lowercase();
        let kind = link.attr("type").unwrap_or_default().to_lowercase();
        if rel.split_whitespace().any(|r| r == "alternate") && FEED_TYPES.contains(&kind.as_str()) {
            if let Some(href) = link.attr("href").and_then(|h| base.join(h.trim()).ok()) {
                return Ok(href.to_string());
            }
        }
    }
    Err(NetError::NoFeedFound)
}

pub fn fetch_feed(client: &Client, url: &str, etag: Option<&str>, modified: Option<&str>) -> Result<FetchResult, NetError> {
    let mut headers = Vec::new();
    if let Some(etag) = etag {
        headers.push(("If-None-Match", etag));
    }
    if let Some(modified) = modified {
        headers.push(("If-Modified-Since", modified));
    }
    let (status, page) = client.get(url, &headers)?;
    if status == 304 {
        return Ok(FetchResult {
            not_modified: true,
            etag: etag.map(str::to_owned),
            modified: modified.map(str::to_owned),
            ..Default::default()
        });
    }
    if status >= 400 {
        return Err(NetError::Status(status));
    }
    let feed = parse_feed(&page.body).map_err(NetError::InvalidFeed)?;
    let current = now();
    let articles = feed.entries.into_iter().map(|entry| entry_to_article(entry, current)).collect();
    let icon_urls = [feed.icon.map(|i| i.uri), feed.logo.map(|i| i.uri)].into_iter().flatten().collect();
    Ok(FetchResult {
        not_modified: false,
        title: feed.title.map(|t| t.content).unwrap_or_default(),
        site_url: preferred_link(&feed.links).unwrap_or_default(),
        etag: page.etag,
        modified: page.modified,
        articles,
        icon_urls,
    })
}

fn entry_to_article(entry: feed_rs::model::Entry, current: f64) -> NewArticle {
    let mut summary = entry.summary.map(|t| t.content).unwrap_or_default();
    if let Some(body) = entry.content.and_then(|c| c.body) {
        if body.len() > summary.len() {
            summary = body;
        }
    }
    let published = entry.published.or(entry.updated).map(|d| d.timestamp() as f64).unwrap_or(current).min(current);
    NewArticle {
        guid: entry.id,
        title: entry.title.map(|t| t.content).unwrap_or_default(),
        link: preferred_link(&entry.links).unwrap_or_default(),
        author: entry.authors.into_iter().find_map(|p| p.name).unwrap_or_default(),
        summary,
        published,
    }
}

pub fn icon_needs_refresh(icon_path: Option<&str>, icon_checked: f64) -> bool {
    let age = now() - icon_checked;
    if icon_path.is_some_and(|p| Path::new(p).exists()) { age > ICON_REFRESH } else { age > ICON_RETRY_FAILED }
}

/// Tenta várias fontes em cascata e grava a primeira imagem decodificável como PNG 64×64.
///
/// Normalizar para PNG aqui é o que evita ícones quebrados na interface: ICO com várias
/// resoluções, SVG e páginas HTML servidas com status 200 são descartados no download.
pub fn resolve_icon(client: &Client, feed_url: &str, site_url: &str, feed_icon_urls: &[String], cache_dir: &Path)
    -> Option<String> {
    let base = if site_url.is_empty() {
        let parsed = Url::parse(feed_url).ok()?;
        format!("{}://{}", parsed.scheme(), parsed.host_str()?)
            + &parsed.port().map(|p| format!(":{p}")).unwrap_or_default()
    } else {
        site_url.to_owned()
    };
    let base_url = Url::parse(&base).ok()?;
    let mut candidates = html_icon_candidates(client, &base);
    candidates.extend(feed_icon_urls.iter().cloned());
    if let Ok(favicon) = base_url.join("/favicon.ico") {
        candidates.push(favicon.to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for url in candidates {
        if !seen.insert(url.clone()) {
            continue;
        }
        if let Some(pixbuf) = download_icon(client, &url) {
            std::fs::create_dir_all(cache_dir).ok()?;
            let name = glib::compute_checksum_for_string(glib::ChecksumType::Sha1, feed_url)?;
            let path = cache_dir.join(format!("{name}.png"));
            pixbuf.savev(&path, "png", &[]).ok()?;
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

fn html_icon_candidates(client: &Client, site_url: &str) -> Vec<String> {
    let Ok(page) = client.get_ok(site_url) else { return Vec::new() };
    let Ok(base) = Url::parse(&page.url) else { return Vec::new() };
    let doc = dom_query::Document::from(page.text());
    let mut scored = Vec::new();
    for link in doc.select("link[rel][href]").nodes() {
        let rel = link.attr("rel").unwrap_or_default().to_lowercase();
        let rels: Vec<&str> = rel.split_whitespace().collect();
        if !rels.iter().any(|r| matches!(*r, "icon" | "apple-touch-icon" | "apple-touch-icon-precomposed")) {
            continue;
        }
        let mut size = link
            .attr("sizes")
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|token| token.split('x').next()?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        if size == 0 && rels.contains(&"apple-touch-icon") {
            size = 180;
        }
        if let Some(href) = link.attr("href").and_then(|h| base.join(h.trim()).ok()) {
            scored.push((size, href.to_string()));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, url)| url).collect()
}

fn download_icon(client: &Client, url: &str) -> Option<Pixbuf> {
    if url.starts_with("data:") {
        return None;
    }
    let (status, page) = client.get(url, &[]).ok()?;
    if status != 200 || page.body.is_empty() || page.content_type.starts_with("text/html") {
        return None;
    }
    let loader = PixbufLoader::new();
    loader.write(&page.body).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    if pixbuf.width() < 8 {
        return None;
    }
    if pixbuf.width() != ICON_SIZE || pixbuf.height() != ICON_SIZE {
        return pixbuf.scale_simple(ICON_SIZE, ICON_SIZE, InterpType::Bilinear);
    }
    Some(pixbuf)
}

pub fn extract_article(client: &Client, url: &str) -> Result<String, NetError> {
    let page = client.get_ok(url)?;
    let mut readability =
        dom_smoothie::Readability::new(page.text(), Some(page.url.as_str()), None).map_err(|_| NetError::ExtractFailed)?;
    let article = readability.parse().map_err(|_| NetError::ExtractFailed)?;
    let content = clean_extracted(&article.content);
    if crate::filters::html_to_text(&content).trim().is_empty() {
        return Err(NetError::ExtractFailed);
    }
    Ok(content)
}

/// Remove classes, estilos e ids do site, que só atrapalham o CSS do leitor.
pub fn clean_extracted(html: &str) -> String {
    let doc = dom_query::Document::fragment(html);
    for node in doc.select("[class], [style], [id]").nodes() {
        node.remove_attrs(&["class", "style", "id"]);
    }
    doc.select("html").inner_html().to_string()
}
