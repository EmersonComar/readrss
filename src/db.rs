//! Persistência em SQLite: feeds, artigos, filtros e preferências.
//!
//! O esquema evolui só por migrações versionadas (`MIGRATIONS`), então bancos criados por
//! qualquer versão anterior (`~/.local/share/readrss/readrss.db`) são atualizados ao abrir.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::filters::{FeedFilter, FilterError, Mode};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS feeds (
    id           INTEGER PRIMARY KEY,
    url          TEXT NOT NULL UNIQUE,
    title        TEXT NOT NULL DEFAULT '',
    site_url     TEXT NOT NULL DEFAULT '',
    icon_path    TEXT,
    icon_checked REAL NOT NULL DEFAULT 0,
    etag         TEXT,
    modified     TEXT,
    last_fetched REAL NOT NULL DEFAULT 0,
    last_error   TEXT
);
CREATE TABLE IF NOT EXISTS articles (
    id           INTEGER PRIMARY KEY,
    feed_id      INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    guid         TEXT NOT NULL,
    title        TEXT NOT NULL DEFAULT '',
    link         TEXT NOT NULL DEFAULT '',
    author       TEXT NOT NULL DEFAULT '',
    summary      TEXT NOT NULL DEFAULT '',
    full_content TEXT,
    published    REAL NOT NULL,
    is_read      INTEGER NOT NULL DEFAULT 0,
    is_starred   INTEGER NOT NULL DEFAULT 0,
    UNIQUE (feed_id, guid)
);
CREATE INDEX IF NOT EXISTS idx_articles_published ON articles(published DESC);
CREATE INDEX IF NOT EXISTS idx_articles_feed ON articles(feed_id, published DESC);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// Aplicadas em ordem; `PRAGMA user_version` guarda quantas já rodaram.
/// Nunca altere uma migração publicada: acrescente outra no fim.
pub const MIGRATIONS: &[&str] = &[r#"
    ALTER TABLE feeds ADD COLUMN custom_title TEXT;
    ALTER TABLE articles ADD COLUMN is_hidden INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE filters (
        id        INTEGER PRIMARY KEY,
        feed_id   INTEGER UNIQUE REFERENCES feeds(id) ON DELETE CASCADE,
        mode      TEXT NOT NULL,
        rules     TEXT NOT NULL,
        use_regex INTEGER NOT NULL DEFAULT 0
    );
"#];

pub const DEFAULT_SETTINGS: &[(&str, &str)] =
    &[("sync_interval_min", "30"), ("notifications", "1"), ("run_in_background", "1"), ("language", "system")];

const FEED_TITLE_SQL: &str = "COALESCE(NULLIF(f.custom_title, ''), NULLIF(f.title, ''), f.url)";
const VISIBLE_SQL: &str = "(a.is_hidden = 0 OR a.is_starred = 1)";
const ARTICLE_COLUMNS: &str = "a.id, a.feed_id, a.title, a.link, a.author, a.summary, a.full_content, a.published, \
                               a.is_read, a.is_starred";

#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub site_url: String,
    pub icon_path: Option<String>,
    pub icon_checked: f64,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub last_error: Option<String>,
    pub custom_title: Option<String>,
    pub unread: i64,
    pub has_filter: bool,
}

impl Feed {
    pub fn display_title(&self) -> &str {
        [self.custom_title.as_deref(), Some(self.title.as_str())]
            .into_iter()
            .flatten()
            .find(|t| !t.is_empty())
            .unwrap_or(&self.url)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Article {
    pub id: i64,
    pub feed_id: i64,
    pub title: String,
    pub link: String,
    pub author: String,
    pub summary: String,
    pub full_content: Option<String>,
    pub published: f64,
    pub is_read: bool,
    pub is_starred: bool,
    pub feed_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewArticle {
    pub guid: String,
    pub title: String,
    pub link: String,
    pub author: String,
    pub summary: String,
    pub published: f64,
}

#[derive(Default)]
pub struct ArticleQuery<'a> {
    pub feed_id: Option<i64>,
    pub unread_only: bool,
    pub starred_only: bool,
    pub search: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DuplicateFeed;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (index, script) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            conn.execute_batch(&format!("BEGIN; {script}; PRAGMA user_version = {}; COMMIT;", index + 1))?;
        }
        for (key, value) in DEFAULT_SETTINGS {
            conn.execute("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)", [key, value])?;
        }
        Ok(Db { conn })
    }

    /// Encerramento limpo: grava o WAL no banco principal e remove os arquivos `-wal`/`-shm`.
    /// O próximo `open` volta a ligar o WAL.
    pub fn finish(&self) {
        let _ = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        let _ = self.conn.pragma_update(None, "journal_mode", "DELETE");
    }

    pub fn user_version(&self) -> i64 {
        self.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap_or(0)
    }

    pub fn get_setting(&self, key: &str) -> String {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
            .or_else(|| DEFAULT_SETTINGS.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
            .unwrap_or_default()
    }

    pub fn set_setting(&self, key: &str, value: &str) {
        let _ = self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        );
    }

    pub fn add_feed(&self, url: &str, title: &str) -> Option<i64> {
        let changed = self.conn.execute("INSERT OR IGNORE INTO feeds (url, title) VALUES (?, ?)", [url, title]).ok()?;
        (changed > 0).then(|| self.conn.last_insert_rowid())
    }

    pub fn remove_feed(&self, feed_id: i64) {
        let _ = self.conn.execute("DELETE FROM feeds WHERE id = ?", [feed_id]);
    }

    pub fn list_feeds(&self) -> Vec<Feed> {
        let sql = format!(
            "SELECT f.*, COUNT(a.id) FILTER (WHERE a.is_read = 0 AND {VISIBLE_SQL}) AS unread,
                    EXISTS (SELECT 1 FROM filters WHERE feed_id = f.id) AS has_filter
             FROM feeds f LEFT JOIN articles a ON a.feed_id = f.id
             GROUP BY f.id ORDER BY lower({FEED_TITLE_SQL})"
        );
        self.query(&sql, [], row_to_feed)
    }

    pub fn get_feed(&self, feed_id: i64) -> Option<Feed> {
        self.conn
            .query_row(
                "SELECT f.*, 0 AS unread, EXISTS (SELECT 1 FROM filters WHERE feed_id = f.id) AS has_filter
                 FROM feeds f WHERE f.id = ?",
                [feed_id],
                row_to_feed,
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn update_feed_url(&self, feed_id: i64, url: &str) -> Result<(), DuplicateFeed> {
        self.conn
            .execute(
                "UPDATE feeds SET url = ?, etag = NULL, modified = NULL, last_error = NULL, icon_checked = 0
                 WHERE id = ?",
                params![url, feed_id],
            )
            .map(|_| ())
            .map_err(|_| DuplicateFeed)
    }

    pub fn set_custom_title(&self, feed_id: i64, title: Option<&str>) {
        let title = title.filter(|t| !t.is_empty());
        let _ = self.conn.execute("UPDATE feeds SET custom_title = ? WHERE id = ?", params![title, feed_id]);
    }

    /// Título e site só são sobrescritos quando o feed informa algo; ETag, Last-Modified e o
    /// erro são gravados como vierem.
    pub fn update_feed_meta(
        &self,
        feed_id: i64,
        title: &str,
        site_url: &str,
        etag: Option<&str>,
        modified: Option<&str>,
        error: Option<&str>,
    ) {
        let _ = self.conn.execute(
            "UPDATE feeds SET title = COALESCE(NULLIF(?, ''), title), site_url = COALESCE(NULLIF(?, ''), site_url),
                              etag = ?, modified = ?, last_error = ?, last_fetched = ?
             WHERE id = ?",
            params![title, site_url, etag, modified, error, now(), feed_id],
        );
    }

    pub fn set_feed_error(&self, feed_id: i64, error: &str) {
        let _ = self
            .conn
            .execute("UPDATE feeds SET last_error = ?, last_fetched = ? WHERE id = ?", params![error, now(), feed_id]);
    }

    pub fn set_feed_icon(&self, feed_id: i64, icon_path: Option<&str>) {
        let _ = self.conn.execute(
            "UPDATE feeds SET icon_path = ?, icon_checked = ? WHERE id = ?",
            params![icon_path, now(), feed_id],
        );
    }

    /// Insere só os inéditos, aplicando o filtro do feed. Retorna quantos novos ficaram
    /// visíveis: é o número que vai para as notificações.
    pub fn insert_articles(&self, feed_id: i64, articles: &[NewArticle]) -> usize {
        let compiled = self.get_filter(feed_id).and_then(|f| f.compile().ok());
        let Ok(tx) = self.conn.unchecked_transaction() else { return 0 };
        let mut visible_new = 0;
        {
            let Ok(mut stmt) = tx.prepare(
                "INSERT OR IGNORE INTO articles (feed_id, guid, title, link, author, summary, published, is_hidden)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            ) else {
                return 0;
            };
            for a in articles {
                let hidden = compiled.as_ref().is_some_and(|c| c.hides(&a.title, &a.summary));
                let inserted = stmt
                    .execute(params![feed_id, a.guid, a.title, a.link, a.author, a.summary, a.published, hidden])
                    .unwrap_or(0);
                if inserted > 0 && !hidden {
                    visible_new += 1;
                }
            }
        }
        let _ = tx.commit();
        visible_new
    }

    pub fn get_filter(&self, feed_id: i64) -> Option<FeedFilter> {
        self.conn
            .query_row("SELECT mode, rules, use_regex FROM filters WHERE feed_id = ?", [feed_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?))
            })
            .optional()
            .ok()
            .flatten()
            .and_then(|(mode, rules, regex)| Some(FeedFilter::from_text(Mode::parse(&mode)?, &rules, regex)))
    }

    /// Grava (ou remove, com `None`) o filtro e reaplica aos artigos do feed.
    /// Retorna quantos artigos do feed ficaram ocultos.
    pub fn set_filter(&self, feed_id: i64, filter: Option<&FeedFilter>) -> Result<usize, FilterError> {
        let compiled = filter.map(FeedFilter::compile).transpose()?;
        let Ok(tx) = self.conn.unchecked_transaction() else { return Ok(0) };
        match filter {
            None => {
                let _ = tx.execute("DELETE FROM filters WHERE feed_id = ?", [feed_id]);
            }
            Some(f) => {
                let _ = tx.execute(
                    "INSERT INTO filters (feed_id, mode, rules, use_regex) VALUES (?, ?, ?, ?)
                     ON CONFLICT(feed_id) DO UPDATE SET mode = excluded.mode, rules = excluded.rules,
                                                        use_regex = excluded.use_regex",
                    params![feed_id, f.mode.as_str(), f.rules_text(), f.use_regex],
                );
            }
        }
        let rows: Vec<(i64, String, String)> = {
            let Ok(mut stmt) = tx.prepare("SELECT id, title, summary FROM articles WHERE feed_id = ?") else {
                return Ok(0);
            };
            stmt.query_map([feed_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map(|rows| rows.filter_map(Result::ok).collect())
                .unwrap_or_default()
        };
        let mut hidden_count = 0;
        for (id, title, summary) in rows {
            let hidden = compiled.as_ref().is_some_and(|c| c.hides(&title, &summary));
            hidden_count += usize::from(hidden);
            let _ = tx.execute("UPDATE articles SET is_hidden = ? WHERE id = ?", params![hidden, id]);
        }
        let _ = tx.commit();
        Ok(hidden_count)
    }

    /// (ocultos, total) que o filtro produziria nos artigos atuais, sem gravar nada.
    pub fn preview_filter(&self, feed_id: i64, filter: &FeedFilter) -> Result<(usize, usize), FilterError> {
        let compiled = filter.compile()?;
        let rows = self.query("SELECT title, summary FROM articles WHERE feed_id = ?", [feed_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        });
        Ok((rows.iter().filter(|(t, s)| compiled.hides(t, s)).count(), rows.len()))
    }

    pub fn list_articles(&self, query: &ArticleQuery) -> Vec<Article> {
        let mut clauses = vec![VISIBLE_SQL.to_string()];
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(feed_id) = query.feed_id {
            clauses.push("a.feed_id = ?".into());
            values.push(feed_id.into());
        }
        if query.unread_only {
            clauses.push("a.is_read = 0".into());
        }
        if query.starred_only {
            clauses.push("a.is_starred = 1".into());
        }
        if !query.search.is_empty() {
            clauses.push("(a.title LIKE ? OR a.summary LIKE ?)".into());
            let pattern = format!("%{}%", query.search);
            values.push(pattern.clone().into());
            values.push(pattern.into());
        }
        let sql = format!(
            "SELECT {ARTICLE_COLUMNS}, {FEED_TITLE_SQL} FROM articles a JOIN feeds f ON f.id = a.feed_id
             WHERE {} ORDER BY a.published DESC LIMIT 500",
            clauses.join(" AND ")
        );
        self.query(&sql, rusqlite::params_from_iter(values), row_to_article)
    }

    pub fn get_article(&self, article_id: i64) -> Option<Article> {
        let sql = format!(
            "SELECT {ARTICLE_COLUMNS}, {FEED_TITLE_SQL} FROM articles a JOIN feeds f ON f.id = a.feed_id WHERE a.id = ?"
        );
        self.conn.query_row(&sql, [article_id], row_to_article).optional().ok().flatten()
    }

    pub fn set_read(&self, article_id: i64, read: bool) {
        let _ = self.conn.execute("UPDATE articles SET is_read = ? WHERE id = ?", params![read, article_id]);
    }

    pub fn set_starred(&self, article_id: i64, starred: bool) {
        let _ = self.conn.execute("UPDATE articles SET is_starred = ? WHERE id = ?", params![starred, article_id]);
    }

    pub fn mark_all_read(&self, feed_id: Option<i64>) {
        let _ = match feed_id {
            None => self.conn.execute("UPDATE articles SET is_read = 1 WHERE is_read = 0", []),
            Some(id) => self.conn.execute("UPDATE articles SET is_read = 1 WHERE is_read = 0 AND feed_id = ?", [id]),
        };
    }

    pub fn set_full_content(&self, article_id: i64, html: &str) {
        let _ = self.conn.execute("UPDATE articles SET full_content = ? WHERE id = ?", params![html, article_id]);
    }

    /// (não lidos visíveis, favoritos)
    pub fn counts(&self) -> (i64, i64) {
        let sql = format!(
            "SELECT COUNT(*) FILTER (WHERE a.is_read = 0 AND {VISIBLE_SQL}), COUNT(*) FILTER (WHERE a.is_starred = 1)
             FROM articles a"
        );
        self.conn.query_row(&sql, [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((0, 0))
    }

    fn query<T, P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
        map: impl FnMut(&Row) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let Ok(mut stmt) = self.conn.prepare(sql) else { return Vec::new() };
        stmt.query_map(params, map).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }
}

fn row_to_feed(r: &Row) -> rusqlite::Result<Feed> {
    Ok(Feed {
        id: r.get("id")?,
        url: r.get("url")?,
        title: r.get("title")?,
        site_url: r.get("site_url")?,
        icon_path: r.get("icon_path")?,
        icon_checked: r.get("icon_checked")?,
        etag: r.get("etag")?,
        modified: r.get("modified")?,
        last_error: r.get("last_error")?,
        custom_title: r.get("custom_title")?,
        unread: r.get("unread")?,
        has_filter: r.get("has_filter")?,
    })
}

fn row_to_article(r: &Row) -> rusqlite::Result<Article> {
    Ok(Article {
        id: r.get(0)?,
        feed_id: r.get(1)?,
        title: r.get(2)?,
        link: r.get(3)?,
        author: r.get(4)?,
        summary: r.get(5)?,
        full_content: r.get(6)?,
        published: r.get(7)?,
        is_read: r.get(8)?,
        is_starred: r.get(9)?,
        feed_title: r.get(10)?,
    })
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempdir::TempDir, Db) {
        let dir = tempdir::TempDir::new();
        let db = Db::open(&dir.path().join("test.db")).unwrap();
        (dir, db)
    }

    fn article(guid: &str, published: f64, title: &str, summary: &str) -> NewArticle {
        NewArticle {
            guid: guid.into(),
            title: title.into(),
            link: format!("https://x.test/{guid}"),
            author: String::new(),
            summary: summary.into(),
            published,
        }
    }

    fn simple(guid: &str) -> NewArticle {
        article(guid, 1000.0, "t", "s")
    }

    fn all(db: &Db) -> Vec<Article> {
        db.list_articles(&ArticleQuery::default())
    }

    #[test]
    fn settings_have_defaults_and_persist() {
        let (_d, db) = db();
        for (key, value) in DEFAULT_SETTINGS {
            assert_eq!(db.get_setting(key), *value);
        }
        db.set_setting("sync_interval_min", "5");
        assert_eq!(db.get_setting("sync_interval_min"), "5");
    }

    #[test]
    fn add_feed_ignores_duplicates_and_title_falls_back_to_url() {
        let (_d, db) = db();
        assert!(db.add_feed("https://a.test/feed", "").is_some());
        assert!(db.add_feed("https://a.test/feed", "").is_none());
        let feeds = db.list_feeds();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].display_title(), "https://a.test/feed");
    }

    #[test]
    fn insert_counts_only_new_and_guid_is_per_feed() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        let b = db.add_feed("https://b.test/feed", "").unwrap();
        assert_eq!(db.insert_articles(a, &[simple("1"), simple("2")]), 2);
        assert_eq!(db.insert_articles(a, &[simple("2"), simple("3")]), 1);
        assert_eq!(db.insert_articles(b, &[simple("1")]), 1);
        assert_eq!(all(&db).len(), 4);
    }

    #[test]
    fn list_filters_order_and_search() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        let b = db.add_feed("https://b.test/feed", "").unwrap();
        db.insert_articles(a, &[article("old", 100.0, "Linux news", "s"), article("new", 300.0, "t", "s")]);
        db.insert_articles(b, &[article("mid", 200.0, "t", "about linux")]);
        assert_eq!(all(&db).iter().map(|x| x.published).collect::<Vec<_>>(), vec![300.0, 200.0, 100.0]);
        assert!(
            db.list_articles(&ArticleQuery { feed_id: Some(b), ..Default::default() }).iter().all(|x| x.feed_id == b)
        );

        let first = all(&db)[0].id;
        db.set_read(first, true);
        db.set_starred(first, true);
        assert!(
            !db.list_articles(&ArticleQuery { unread_only: true, ..Default::default() }).iter().any(|x| x.id == first)
        );
        assert_eq!(db.list_articles(&ArticleQuery { starred_only: true, ..Default::default() })[0].id, first);
        let mut found: Vec<_> = db
            .list_articles(&ArticleQuery { search: "linux", ..Default::default() })
            .into_iter()
            .map(|x| x.title)
            .collect();
        found.sort();
        assert_eq!(found, vec!["Linux news", "t"]);
    }

    #[test]
    fn counts_and_mark_all_read() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        let b = db.add_feed("https://b.test/feed", "").unwrap();
        db.insert_articles(a, &[simple("1"), simple("2")]);
        db.insert_articles(b, &[simple("3")]);
        assert_eq!(db.counts(), (3, 0));
        db.mark_all_read(Some(a));
        assert_eq!(db.counts().0, 1);
        db.mark_all_read(None);
        assert_eq!(db.counts().0, 0);
    }

    #[test]
    fn remove_feed_cascades() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        db.insert_articles(a, &[simple("1")]);
        db.set_filter(a, Some(&FeedFilter::from_text(Mode::Hide, "x", false))).unwrap();
        db.remove_feed(a);
        assert!(db.list_feeds().is_empty() && all(&db).is_empty());
        assert_eq!(db.conn.query_row("SELECT COUNT(*) FROM filters", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn meta_keeps_title_when_empty_and_errors_are_cleared() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "Imported").unwrap();
        db.update_feed_meta(a, "", "", Some("e1"), Some("m1"), None);
        let feed = db.get_feed(a).unwrap();
        assert_eq!(
            (feed.title.as_str(), feed.etag.as_deref(), feed.modified.as_deref()),
            ("Imported", Some("e1"), Some("m1"))
        );
        db.set_feed_error(a, "HTTP 500");
        assert_eq!(db.get_feed(a).unwrap().last_error.as_deref(), Some("HTTP 500"));
        db.update_feed_meta(a, "Real", "https://a.test/", None, None, None);
        let feed = db.get_feed(a).unwrap();
        assert_eq!((feed.title.as_str(), feed.last_error), ("Real", None));
    }

    #[test]
    fn full_content_is_cached() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        db.insert_articles(a, &[simple("1")]);
        let id = all(&db)[0].id;
        db.set_full_content(id, "<p>full</p>");
        assert_eq!(db.get_article(id).unwrap().full_content.as_deref(), Some("<p>full</p>"));
    }

    #[test]
    fn migrates_database_from_initial_version() {
        let dir = tempdir::TempDir::new();
        let path = dir.path().join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute("INSERT INTO feeds (url, title) VALUES ('https://a.test/feed', 'A')", []).unwrap();
            conn.execute(
                "INSERT INTO articles (feed_id, guid, title, published, is_starred) VALUES (1, 'g', 't', 1, 1)",
                [],
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.user_version(), MIGRATIONS.len() as i64);
        let feed = &db.list_feeds()[0];
        assert_eq!((feed.display_title(), feed.has_filter), ("A", false));
        assert!(all(&db)[0].is_starred);
        drop(db);
        Db::open(&path).unwrap();
    }

    #[test]
    fn custom_title_survives_sync_and_can_be_cleared() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "Feed title").unwrap();
        db.insert_articles(a, &[simple("1")]);
        db.set_custom_title(a, Some("Meu nome"));
        db.update_feed_meta(a, "Feed title v2", "", None, None, None);
        assert_eq!(db.get_feed(a).unwrap().display_title(), "Meu nome");
        assert_eq!(all(&db)[0].feed_title, "Meu nome");
        db.set_custom_title(a, Some(""));
        assert_eq!(db.get_feed(a).unwrap().display_title(), "Feed title v2");
    }

    #[test]
    fn update_feed_url_resets_state_and_rejects_duplicates() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        db.add_feed("https://b.test/feed", "").unwrap();
        db.update_feed_meta(a, "", "", Some("\"v1\""), Some("Mon"), Some("HTTP 404"));
        db.set_feed_icon(a, Some("/tmp/icon.png"));
        db.update_feed_url(a, "https://a.test/new").unwrap();
        let feed = db.get_feed(a).unwrap();
        assert_eq!(feed.url, "https://a.test/new");
        assert_eq!((feed.etag, feed.modified, feed.last_error, feed.icon_checked), (None, None, None, 0.0));
        assert_eq!(feed.icon_path.as_deref(), Some("/tmp/icon.png"));
        assert_eq!(db.update_feed_url(a, "https://b.test/feed"), Err(DuplicateFeed));
        assert_eq!(db.get_feed(a).unwrap().url, "https://a.test/new");
    }

    #[test]
    fn filter_hides_new_articles_and_they_do_not_count() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        db.set_filter(a, Some(&FeedFilter::from_text(Mode::Hide, "BBB", false))).unwrap();
        let new = db.insert_articles(a, &[article("1", 1.0, "Paredão do BBB", ""), article("2", 2.0, "Ubuntu", "")]);
        assert_eq!(new, 1);
        assert_eq!(all(&db).iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["Ubuntu"]);
        assert_eq!(db.counts().0, 1);
        let feed = &db.list_feeds()[0];
        assert_eq!((feed.unread, feed.has_filter), (1, true));
    }

    #[test]
    fn changing_filter_reapplies_and_round_trips() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        db.insert_articles(a, &[article("1", 1.0, "Ubuntu 26.10", ""), article("2", 2.0, "Windows 12", "")]);
        assert_eq!(db.set_filter(a, Some(&FeedFilter::from_text(Mode::Only, "ubuntu", false))), Ok(1));
        assert_eq!(all(&db)[0].title, "Ubuntu 26.10");
        assert_eq!(db.set_filter(a, Some(&FeedFilter::from_text(Mode::Hide, "ubuntu", false))), Ok(1));
        assert_eq!(all(&db)[0].title, "Windows 12");
        assert_eq!(db.set_filter(a, None), Ok(0));
        assert_eq!(all(&db).len(), 2);
        assert_eq!(db.get_filter(a), None);
        let f = FeedFilter::from_text(Mode::Only, "a|b\nc", true);
        db.set_filter(a, Some(&f)).unwrap();
        assert_eq!(db.get_filter(a), Some(f));
    }

    #[test]
    fn starred_are_never_hidden_and_filters_are_per_feed() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        let b = db.add_feed("https://b.test/feed", "").unwrap();
        db.insert_articles(a, &[article("1", 1.0, "BBB", "")]);
        db.insert_articles(b, &[article("2", 2.0, "BBB", "")]);
        db.set_starred(all(&db).iter().find(|x| x.feed_id == a).unwrap().id, true);
        db.set_filter(a, Some(&FeedFilter::from_text(Mode::Hide, "BBB", false))).unwrap();
        db.insert_articles(a, &[article("3", 3.0, "BBB de novo", "")]);
        let titles: Vec<_> = all(&db).into_iter().map(|x| (x.feed_id, x.title)).collect();
        assert_eq!(titles, vec![(b, "BBB".to_string()), (a, "BBB".to_string())]);
    }

    #[test]
    fn preview_changes_nothing_and_invalid_filter_is_not_saved() {
        let (_d, db) = db();
        let a = db.add_feed("https://a.test/feed", "").unwrap();
        db.insert_articles(a, &[article("1", 1.0, "BBB", ""), article("2", 2.0, "x", ""), article("3", 3.0, "y", "")]);
        assert_eq!(db.preview_filter(a, &FeedFilter::from_text(Mode::Hide, "BBB", false)), Ok((1, 3)));
        assert_eq!(db.get_filter(a), None);
        assert!(db.set_filter(a, Some(&FeedFilter::from_text(Mode::Hide, "(", true))).is_err());
        assert_eq!(db.get_filter(a), None);
        assert_eq!(all(&db).len(), 3);
    }
}

#[cfg(test)]
pub mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> TempDir {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!("readrss-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Default for TempDir {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
