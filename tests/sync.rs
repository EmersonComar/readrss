mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use common::{FakeWeb, Response, TempDir, image_bytes, rss, simple_item};
use gtk::glib;
use readrss::db::{ArticleQuery, Db};
use readrss::sync::{SyncReport, SyncService};

struct Fixture {
    _dir: TempDir,
    db: Rc<Db>,
    service: Rc<SyncService>,
}

fn fixture() -> Fixture {
    let dir = TempDir::new();
    let db = Rc::new(Db::open(&dir.path().join("t.db")).unwrap());
    let service = Rc::new(SyncService::new(dir.path().join("icons")));
    Fixture { _dir: dir, db, service }
}

/// Roda uma sincronização num contexto GLib próprio da thread do teste e devolve o relatório.
fn sync(f: &Fixture, ids: Option<Vec<i64>>) -> SyncReport {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let report: Rc<RefCell<Option<SyncReport>>> = Rc::default();
            let slot = report.clone();
            assert!(f.service.sync(&f.db, ids, move |r| *slot.borrow_mut() = Some(r)));
            assert!(f.service.busy());
            assert!(!f.service.sync(&f.db, None, |_| {}), "sincronização simultânea deve ser recusada");
            let deadline = Instant::now() + Duration::from_secs(20);
            while report.borrow().is_none() && Instant::now() < deadline {
                context.iteration(true);
            }
            assert!(!f.service.busy());
            report.take().expect("relatório")
        })
        .unwrap()
}

#[test]
fn sync_inserts_then_uses_etag_and_reports_errors() {
    let web = FakeWeb::start();
    let body = rss(
        &(simple_item("1", "One", "https://a.test/1") + &simple_item("2", "Two", "https://a.test/2")),
        "Feed A",
        &web.url("/"),
        "",
    );
    web.route("/a", move |req| {
        if req.headers.get("if-none-match").map(String::as_str) == Some("\"v1\"") {
            Response::new(304, "text/plain", "")
        } else {
            Response::new(200, "application/rss+xml", body.clone()).header("ETag", "\"v1\"")
        }
    });
    web.add("/b", Response::new(500, "text/plain", "boom"));
    let f = fixture();
    let a = f.db.add_feed(&web.url("/a"), "").unwrap();
    let b = f.db.add_feed(&web.url("/b"), "").unwrap();

    let report = sync(&f, None);
    assert_eq!(report.new_articles, 2);
    assert_eq!(report.new_by_feed, vec![("Feed A".to_owned(), 2)], "título do feed, não a URL");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].1, "HTTP 500 Internal Server Error");
    assert_eq!(f.db.get_feed(b).unwrap().last_error.as_deref(), Some("HTTP 500 Internal Server Error"));
    assert_eq!(f.db.get_feed(a).unwrap().etag.as_deref(), Some("\"v1\""));

    let again = sync(&f, Some(vec![a]));
    assert_eq!(again.new_articles, 0);
    assert_eq!(f.db.list_articles(&ArticleQuery::default()).len(), 2);
    assert!(!web.paths()[3..].contains(&"/b".to_owned()), "sync de feeds específicos não toca nos outros");
}

#[test]
fn sync_resolves_icon_and_fills_untitled() {
    let web = FakeWeb::start();
    let items = common::item("x", "", "https://a.test/x", "s", "", "");
    web.add("/feed", Response::new(200, "application/rss+xml", rss(&items, "F", &web.url("/"), "")));
    web.add("/favicon.ico", Response::new(200, "image/x-icon", image_bytes("ico", 32)));
    let f = fixture();
    let id = f.db.add_feed(&web.url("/feed"), "").unwrap();
    sync(&f, None);
    let feed = f.db.get_feed(id).unwrap();
    assert!(feed.icon_path.is_some_and(|p| std::path::Path::new(&p).exists()));
    assert!(feed.icon_checked > 0.0);
    assert_eq!(f.db.list_articles(&ArticleQuery::default())[0].title, "(untitled)");
}
