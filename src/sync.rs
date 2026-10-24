//! Sincronização dos feeds: rede em threads, gravação no banco na thread da interface.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gtk::{gio, glib};

use crate::db::{Db, Feed};
use crate::i18n::gettext;
use crate::net::{self, Client, FetchResult, NetError};

pub const MAX_WORKERS: usize = 6;

#[derive(Clone, Debug, Default)]
pub struct SyncReport {
    pub new_articles: usize,
    pub new_by_feed: Vec<(String, usize)>,
    pub errors: Vec<(String, String)>,
}

struct Job {
    url: String,
    etag: Option<String>,
    modified: Option<String>,
    site_url: String,
    needs_icon: bool,
}

struct Outcome {
    fetched: Result<FetchResult, NetError>,
    icon: Option<Option<String>>,
}

pub struct SyncService {
    running: Cell<bool>,
    stopping: Arc<AtomicBool>,
    icon_dir: PathBuf,
}

impl SyncService {
    pub fn new(icon_dir: PathBuf) -> SyncService {
        SyncService { running: Cell::new(false), stopping: Arc::new(AtomicBool::new(false)), icon_dir }
    }

    pub fn busy(&self) -> bool {
        self.running.get()
    }

    /// Nenhum feed novo começa depois disso (usado ao sair do app).
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// Dispara a sincronização; `false` se já houver uma em andamento.
    pub fn sync(self: &Rc<Self>, db: &Rc<Db>, feed_ids: Option<Vec<i64>>, on_done: impl FnOnce(SyncReport) + 'static)
        -> bool {
        if self.running.replace(true) {
            return false;
        }
        let feeds: Vec<Feed> = db
            .list_feeds()
            .into_iter()
            .filter(|f| feed_ids.as_ref().is_none_or(|ids| ids.contains(&f.id)))
            .collect();
        let jobs: Vec<Job> = feeds
            .iter()
            .map(|f| Job {
                url: f.url.clone(),
                etag: f.etag.clone(),
                modified: f.modified.clone(),
                site_url: f.site_url.clone(),
                needs_icon: net::icon_needs_refresh(f.icon_path.as_deref(), f.icon_checked),
            })
            .collect();
        let stopping = self.stopping.clone();
        let icon_dir = self.icon_dir.clone();
        let service = Rc::clone(self);
        let db = Rc::clone(db);
        glib::spawn_future_local(async move {
            let outcomes = gio::spawn_blocking(move || run_jobs(&jobs, &icon_dir, &stopping)).await.unwrap_or_default();
            let report = apply(&db, &feeds, outcomes);
            service.running.set(false);
            on_done(report);
        });
        true
    }
}

fn run_jobs(jobs: &[Job], icon_dir: &std::path::Path, stopping: &AtomicBool) -> Vec<Option<Outcome>> {
    let client = Client::new();
    let next = AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Outcome>>> = jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..MAX_WORKERS.min(jobs.len()) {
            scope.spawn(|| {
                while !stopping.load(Ordering::SeqCst) {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(job) = jobs.get(index) else { break };
                    let fetched = net::fetch_feed(&client, &job.url, job.etag.as_deref(), job.modified.as_deref());
                    let icon = match &fetched {
                        Ok(result) if job.needs_icon => {
                            let site = if result.site_url.is_empty() { &job.site_url } else { &result.site_url };
                            Some(net::resolve_icon(&client, &job.url, site, &result.icon_urls, icon_dir))
                        }
                        _ => None,
                    };
                    *results[index].lock().unwrap() = Some(Outcome { fetched, icon });
                }
            });
        }
    });
    results.into_iter().map(|cell| cell.into_inner().unwrap()).collect()
}

fn apply(db: &Db, feeds: &[Feed], outcomes: Vec<Option<Outcome>>) -> SyncReport {
    let mut report = SyncReport::default();
    for (feed, outcome) in feeds.iter().zip(outcomes) {
        let Some(outcome) = outcome else { continue };
        let new = match outcome.fetched {
            Err(error) => {
                db.set_feed_error(feed.id, &error.describe());
                let title = db.get_feed(feed.id).map(|f| f.display_title().to_owned()).unwrap_or_default();
                report.errors.push((title, error.describe()));
                continue;
            }
            Ok(result) if result.not_modified => {
                db.update_feed_meta(feed.id, "", "", result.etag.as_deref(), result.modified.as_deref(), None);
                0
            }
            Ok(mut result) => {
                db.update_feed_meta(feed.id, &result.title, &result.site_url, result.etag.as_deref(),
                                    result.modified.as_deref(), None);
                let untitled = gettext("(untitled)");
                for article in &mut result.articles {
                    if article.title.is_empty() {
                        article.title = untitled.clone();
                        if article.guid.is_empty() {
                            article.guid = untitled.clone();
                        }
                    }
                }
                db.insert_articles(feed.id, &result.articles)
            }
        };
        if let Some(icon) = outcome.icon {
            db.set_feed_icon(feed.id, icon.as_deref().or(feed.icon_path.as_deref()));
        }
        if new > 0 {
            let title = db.get_feed(feed.id).map(|f| f.display_title().to_owned()).unwrap_or_default();
            report.new_articles += new;
            report.new_by_feed.push((title, new));
        }
    }
    report
}
