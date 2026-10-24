#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};

#[derive(Clone, Debug)]
pub struct Request {
    pub path: String,
    pub headers: HashMap<String, String>,
}

#[derive(Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Response {
        Response { status, content_type: content_type.into(), headers: vec![], body: body.into() }
    }

    pub fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.into(), value.into()));
        self
    }
}

type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

/// Servidor HTTP falso numa porta local: os testes não dependem de nenhum site no ar.
pub struct FakeWeb {
    pub base: String,
    routes: Arc<Mutex<HashMap<String, Handler>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl FakeWeb {
    pub fn start() -> FakeWeb {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<Mutex<HashMap<String, Handler>>> = Arc::default();
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let (r, q) = (routes.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (r, q) = (r.clone(), q.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let mut headers = HashMap::new();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = header.split_once(':') {
                            headers.insert(k.trim().to_lowercase(), v.trim().to_owned());
                        }
                    }
                    let request = Request { path: path.clone(), headers };
                    q.lock().unwrap().push(request.clone());
                    let handler = r.lock().unwrap().get(&path).cloned();
                    let response =
                        handler.map(|h| h(&request)).unwrap_or_else(|| Response::new(404, "text/plain", "not found"));
                    let mut out = stream;
                    let mut head = format!(
                        "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n",
                        response.status,
                        response.content_type,
                        response.body.len()
                    );
                    for (k, v) in &response.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    head.push_str("\r\n");
                    let _ = out.write_all(head.as_bytes());
                    let _ = out.write_all(&response.body);
                    let _ = reader.read(&mut [0u8; 0]);
                });
            }
        });
        FakeWeb { base, routes, requests }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub fn add(&self, path: &str, response: Response) {
        self.route(path, move |_| response.clone());
    }

    pub fn route(&self, path: &str, handler: impl Fn(&Request) -> Response + Send + Sync + 'static) {
        self.routes.lock().unwrap().insert(path.into(), Arc::new(handler));
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.requests().into_iter().map(|r| r.path).collect()
    }
}

pub fn image_bytes(format: &str, size: i32) -> Vec<u8> {
    let pixbuf = Pixbuf::new(Colorspace::Rgb, true, 8, size, size).unwrap();
    pixbuf.fill(0x3366ccff);
    pixbuf.save_to_bufferv(format, &[]).unwrap()
}

pub fn rss(items: &str, title: &str, link: &str, image: &str) -> String {
    let image = if image.is_empty() {
        String::new()
    } else {
        format!("<image><url>{image}</url><title>x</title><link>{link}</link></image>")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel>
<title>{title}</title><link>{link}</link><description>d</description>{image}{items}
</channel></rss>"#
    )
}

pub fn item(guid: &str, title: &str, link: &str, summary: &str, content: &str, date: &str) -> String {
    let guid_xml = if guid.is_empty() { String::new() } else { format!("<guid>{guid}</guid>") };
    let content_xml = if content.is_empty() {
        String::new()
    } else {
        format!("<content:encoded><![CDATA[{content}]]></content:encoded>")
    };
    let date_xml = if date.is_empty() { String::new() } else { format!("<pubDate>{date}</pubDate>") };
    format!(
        "<item><title>{title}</title><link>{link}</link>{guid_xml}<description><![CDATA[{summary}]]></description>{content_xml}{date_xml}</item>"
    )
}

pub fn simple_item(guid: &str, title: &str, link: &str) -> String {
    item(guid, title, link, "Summary", "", "Mon, 19 Oct 2026 10:00:00 GMT")
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("readrss-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
