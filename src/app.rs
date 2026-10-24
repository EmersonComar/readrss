//! Aplicação: ações globais, agendamento da sincronização, notificações e modo serviço.

use std::cell::{Cell, RefCell};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::about;
use crate::constants::{APP_ID, icons_dir};
use crate::db::{Db, Feed};
use crate::i18n::{self, fill, gettext, ngettext, ngettext_n};
use crate::net;
use crate::opml;
use crate::sync::{SyncReport, SyncService};
use crate::window::{Source, Window};

pub const SYNC_INTERVALS: [u32; 3] = [5, 30, 60];

type AppAction = (&'static str, fn(&Rc<App>), &'static [&'static str]);

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub app_id: String,
}

impl Config {
    /// Padrão do usuário; `READRSS_DATA_DIR`, `READRSS_CACHE_DIR` e `READRSS_APP_ID` permitem
    /// rodar isolado (testes e desenvolvimento ao lado da versão instalada).
    pub fn from_env() -> Config {
        let dir = |var: &str, default: PathBuf| std::env::var_os(var).map(PathBuf::from).unwrap_or(default);
        Config {
            data_dir: dir("READRSS_DATA_DIR", glib::user_data_dir().join("readrss")),
            cache_dir: dir("READRSS_CACHE_DIR", glib::user_cache_dir().join("readrss")),
            app_id: std::env::var("READRSS_APP_ID").unwrap_or_else(|_| APP_ID.to_owned()),
        }
    }
}

pub struct App {
    pub application: adw::Application,
    pub db: Rc<Db>,
    pub sync: Rc<SyncService>,
    window: RefCell<Option<Rc<Window>>>,
    timer: RefCell<Option<glib::SourceId>>,
    started: Cell<bool>,
    background: Cell<bool>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

fn interval_label(minutes: u32) -> String {
    if minutes.is_multiple_of(60) {
        ngettext_n("{n} hour", "{n} hours", (minutes / 60) as usize)
    } else {
        ngettext_n("{n} minute", "{n} minutes", minutes as usize)
    }
}

fn new_articles_text(n: usize) -> String {
    ngettext_n("{n} new article", "{n} new articles", n)
}

impl App {
    pub fn new(config: Config) -> Rc<App> {
        let db = Rc::new(Db::open(&config.data_dir.join("readrss.db")).expect("não foi possível abrir o banco"));
        i18n::init(&db.get_setting("language"));
        let application = adw::Application::builder().application_id(&config.app_id).build();
        application.add_main_option(
            "background",
            glib::Char::from(0u8),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            &gettext("Start in the background without opening the window (used by the systemd service)"),
            None,
        );
        let app = Rc::new(App {
            application,
            db,
            sync: Rc::new(SyncService::new(config.cache_dir.join("icons"))),
            window: RefCell::new(None),
            timer: RefCell::new(None),
            started: Cell::new(false),
            background: Cell::new(false),
            hold: RefCell::new(None),
        });
        app.connect();
        app
    }

    pub fn run(self: &Rc<Self>, args: &[String]) -> glib::ExitCode {
        self.application.run_with_args(args)
    }

    pub fn window(&self) -> Option<Rc<Window>> {
        self.window.borrow().clone()
    }

    fn connect(self: &Rc<Self>) {
        let app = self.clone();
        self.application.connect_handle_local_options(move |application, options| {
            if options.contains("background") {
                let _ = application.register(gio::Cancellable::NONE);
                if application.is_remote() {
                    return ControlFlow::Break(glib::ExitCode::SUCCESS);
                }
                app.background.set(true);
            }
            ControlFlow::Continue(())
        });
        let app = self.clone();
        self.application.connect_startup(move |_| app.startup());
        let app = self.clone();
        self.application.connect_activate(move |_| app.activate());
        let app = self.clone();
        self.application.connect_shutdown(move |_| {
            if let Some(source) = app.timer.borrow_mut().take() {
                source.remove();
            }
            app.sync.stop();
            app.db.finish();
        });
    }

    fn startup(self: &Rc<Self>) {
        if let Some(display) = gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_search_path(icons_dir());
        }
        gtk::Window::set_default_icon_name(APP_ID);
        for signal in [2, 15] {
            let application = self.application.clone();
            glib::spawn_future_local(async move {
                glib_unix::signal_future(signal).await;
                application.quit();
            });
        }

        let actions: [AppAction; 9] = [
            ("add-feed", |a| a.show_add_feed(), &["<Control>n"]),
            ("sync", |a| a.start_sync(true, None), &["<Control>r", "F5"]),
            ("import-opml", |a| a.import_opml(), &[]),
            ("export-opml", |a| a.export_opml(), &[]),
            (
                "preferences",
                |a| {
                    a.show_preferences();
                },
                &["<Control>comma"],
            ),
            (
                "changelog",
                |a| {
                    if let Some(w) = a.window() {
                        about::show_changelog(&w.window);
                    }
                },
                &[],
            ),
            (
                "about",
                |a| {
                    if let Some(w) = a.window() {
                        about::show_about(&w.window);
                    }
                },
                &[],
            ),
            ("show-window", |a| a.application.activate(), &[]),
            ("quit", |a| a.application.quit(), &["<Control>q"]),
        ];
        for (name, handler, accels) in actions {
            let action = gio::SimpleAction::new(name, None);
            let app = self.clone();
            action.connect_activate(move |_, _| handler(&app));
            self.application.add_action(&action);
            if !accels.is_empty() {
                self.application.set_accels_for_action(&format!("app.{name}"), accels);
            }
        }
    }

    fn activate(self: &Rc<Self>) {
        if !self.started.replace(true) {
            self.reschedule_sync();
            self.start_sync(false, None);
        }
        if self.background.replace(false) {
            *self.hold.borrow_mut() = Some(self.application.hold());
            return;
        }
        self.present_window();
    }

    pub fn present_window(self: &Rc<Self>) {
        let window = self.window().unwrap_or_else(|| {
            let window = Window::new(self, Source::Unread);
            *self.window.borrow_mut() = Some(window.clone());
            self.apply_background_setting();
            window.set_syncing(self.sync.busy());
            self.hold.borrow_mut().take();
            window
        });
        window.present();
    }

    fn apply_background_setting(&self) {
        if let Some(window) = self.window() {
            window.window.set_hide_on_close(self.db.get_setting("run_in_background") == "1");
        }
    }

    pub fn reschedule_sync(self: &Rc<Self>) {
        if let Some(source) = self.timer.borrow_mut().take() {
            source.remove();
        }
        let minutes: u64 = self.db.get_setting("sync_interval_min").parse().unwrap_or(30);
        let app = Rc::downgrade(self);
        let source = glib::timeout_add_seconds_local((minutes * 60) as u32, move || {
            if let Some(app) = app.upgrade() {
                app.start_sync(false, None);
            }
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(source);
    }

    pub fn start_sync(self: &Rc<Self>, manual: bool, feed_ids: Option<Vec<i64>>) {
        let app = self.clone();
        let started = self.sync.sync(&self.db, feed_ids, move |report| app.sync_done(report, manual));
        let Some(window) = self.window() else { return };
        if started {
            window.set_syncing(true);
        } else if manual {
            window.toast(&gettext("A sync is already in progress"));
        }
    }

    fn sync_done(self: &Rc<Self>, report: SyncReport, manual: bool) {
        let notifications = self.db.get_setting("notifications") == "1";
        let Some(window) = self.window() else {
            if report.new_articles > 0 && notifications {
                self.notify(&report);
            }
            return;
        };
        window.set_syncing(false);
        window.refresh_after_sync();
        if report.new_articles > 0 {
            if window.window.is_active() {
                window.toast(&new_articles_text(report.new_articles));
            } else if notifications {
                self.notify(&report);
            }
        } else if manual {
            window.toast(&gettext("No new articles"));
        }
        if manual && !report.errors.is_empty() {
            let n = report.errors.len();
            window.toast(&fill(
                &ngettext(
                    "{n} feed failed (hover the warning icon for details)",
                    "{n} feeds failed (hover the warning icons for details)",
                    n as u32,
                ),
                &[("n", &n)],
            ));
        }
    }

    fn notify(&self, report: &SyncReport) {
        let mut top = report.new_by_feed.clone();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        let mut body = top.iter().take(4).map(|(title, n)| format!("{title} ({n})")).collect::<Vec<_>>().join(", ");
        if top.len() > 4 {
            body += &fill(&gettext(" and {n} more"), &[("n", &(top.len() - 4))]);
        }
        let notification = gio::Notification::new(&new_articles_text(report.new_articles));
        notification.set_body(Some(&body));
        notification.set_default_action("app.show-window");
        self.application.send_notification(Some("new-articles"), &notification);
    }

    fn toast(&self, text: &str) {
        if let Some(window) = self.window() {
            window.toast(text);
        }
    }

    fn show_add_feed(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let entry = gtk::Entry::builder()
            .placeholder_text(gettext("https://site.com or https://site.com/feed"))
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Add feed"))
            .body(gettext("Enter the URL of the feed or the website; the feed is detected automatically."))
            .extra_child(&entry)
            .default_response("add")
            .close_response("cancel")
            .build();
        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.add_response("add", &gettext("Add"));
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        let app = self.clone();
        let input = entry.clone();
        dialog.connect_response(None, move |_, response| {
            let url = input.text().trim().to_owned();
            if response == "add" && !url.is_empty() {
                app.add_feed(url);
            }
        });
        dialog.present(Some(&window.window));
        entry.grab_focus();
    }

    pub fn add_feed(self: &Rc<Self>, url: String) {
        self.toast(&gettext("Looking for the feed…"));
        let app = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || net::discover_feed_url(&net::Client::new(), &url))
                .await
                .unwrap_or(Err(net::NetError::NoFeedFound));
            match result {
                Ok(feed_url) => match app.db.add_feed(&feed_url, "") {
                    Some(feed_id) => {
                        if let Some(window) = app.window() {
                            window.reload_sidebar();
                        }
                        app.start_sync(false, Some(vec![feed_id]));
                    }
                    None => app.toast(&gettext("This feed is already subscribed")),
                },
                Err(error) => {
                    app.toast(&fill(&gettext("Could not add the feed: {error}"), &[("error", &error.describe())]));
                }
            }
        });
    }

    /// Salva o nome na hora; a URL nova é validada (descoberta) em segundo plano antes de gravar.
    pub fn edit_feed(self: &Rc<Self>, feed: &Feed, url: &str, name: &str) {
        self.db.set_custom_title(feed.id, Some(name));
        if url == feed.url {
            self.after_feed_edit(true);
            return;
        }
        self.toast(&gettext("Checking the new address…"));
        let (app, feed_id, old_url, url) = (self.clone(), feed.id, feed.url.clone(), url.to_owned());
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || net::discover_feed_url(&net::Client::new(), &url))
                .await
                .unwrap_or(Err(net::NetError::NoFeedFound));
            match result {
                Ok(feed_url) => {
                    if feed_url != old_url && app.db.update_feed_url(feed_id, &feed_url).is_err() {
                        app.toast(&gettext("This address already belongs to another feed"));
                        app.after_feed_edit(false);
                        return;
                    }
                    app.after_feed_edit(true);
                    app.start_sync(false, Some(vec![feed_id]));
                }
                Err(error) => {
                    app.toast(&fill(&gettext("The address was not changed: {error}"), &[("error", &error.describe())]));
                    app.after_feed_edit(false);
                }
            }
        });
    }

    fn after_feed_edit(&self, toast: bool) {
        let Some(window) = self.window() else { return };
        window.reload_sidebar();
        window.reload_articles(true);
        if toast {
            window.toast(&gettext("Feed updated"));
        }
    }

    fn import_opml(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let filter = opml_filter();
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog =
            gtk::FileDialog::builder().title(gettext("Import OPML")).default_filter(&filter).filters(&filters).build();
        let app = self.clone();
        dialog.open(Some(&window.window), gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else { return };
            let entries = match opml::parse_opml(&path) {
                Ok(entries) => entries,
                Err(error) => {
                    app.toast(&fill(&gettext("Invalid OPML: {error}"), &[("error", &error)]));
                    return;
                }
            };
            let added: Vec<i64> = entries.iter().filter_map(|(url, title)| app.db.add_feed(url, title)).collect();
            if let Some(window) = app.window() {
                window.reload_sidebar();
            }
            let text = ngettext(
                "{n} feed imported ({skipped} already existed)",
                "{n} feeds imported ({skipped} already existed)",
                added.len() as u32,
            );
            app.toast(&fill(&text, &[("n", &added.len()), ("skipped", &(entries.len() - added.len()))]));
            if !added.is_empty() {
                app.start_sync(false, Some(added));
            }
        });
    }

    fn export_opml(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let dialog = gtk::FileDialog::builder().title(gettext("Export OPML")).initial_name("readrss.opml").build();
        let app = self.clone();
        dialog.save(Some(&window.window), gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else { return };
            let feeds = app.db.list_feeds();
            match opml::write_opml(&path, &feeds) {
                Ok(()) => app.toast(&ngettext_n("{n} feed exported", "{n} feeds exported", feeds.len())),
                Err(error) => app.toast(&error.to_string()),
            }
        });
    }

    pub fn show_preferences(self: &Rc<Self>) -> Option<adw::PreferencesDialog> {
        let window = self.window()?;
        let page = adw::PreferencesPage::new();
        let dialog = adw::PreferencesDialog::builder().title(gettext("Preferences")).build();

        let general = adw::PreferencesGroup::builder().title(gettext("General")).build();
        let codes: Vec<&str> = std::iter::once(i18n::SYSTEM).chain(i18n::LANGUAGES.iter().map(|(c, _)| *c)).collect();
        let system_default = gettext("System default");
        let labels: Vec<&str> =
            std::iter::once(system_default.as_str()).chain(i18n::LANGUAGES.iter().map(|(_, l)| *l)).collect();
        let language =
            adw::ComboRow::builder().title(gettext("Language")).model(&gtk::StringList::new(&labels)).build();
        let current = self.db.get_setting("language");
        language.set_selected(codes.iter().position(|c| *c == current).unwrap_or(0) as u32);
        let app = self.clone();
        let dialog_ref = dialog.downgrade();
        language.connect_selected_notify(move |row| {
            let code = codes[row.selected() as usize];
            if let Some(dialog) = dialog_ref.upgrade() {
                app.change_language(code, &dialog);
            }
        });
        general.add(&language);
        page.add(&general);

        let group = adw::PreferencesGroup::builder().title(gettext("Sync")).build();
        let intervals: Vec<String> = SYNC_INTERVALS.iter().map(|m| interval_label(*m)).collect();
        let interval_refs: Vec<&str> = intervals.iter().map(String::as_str).collect();
        let interval = adw::ComboRow::builder()
            .title(gettext("Update interval"))
            .model(&gtk::StringList::new(&interval_refs))
            .build();
        let minutes: u32 = self.db.get_setting("sync_interval_min").parse().unwrap_or(30);
        interval.set_selected(SYNC_INTERVALS.iter().position(|m| *m == minutes).unwrap_or(1) as u32);
        let app = self.clone();
        interval.connect_selected_notify(move |row| {
            let minutes = SYNC_INTERVALS[row.selected() as usize];
            app.db.set_setting("sync_interval_min", &minutes.to_string());
            app.reschedule_sync();
        });
        group.add(&interval);
        group.add(&self.switch_row(
            "notifications",
            &gettext("Notifications"),
            &gettext("Notify about new articles when the window is not focused"),
            false,
        ));
        group.add(&self.switch_row(
            "run_in_background",
            &gettext("Run in background"),
            &gettext("Keep syncing after the window is closed (use “Quit” to exit)"),
            true,
        ));
        page.add(&group);

        dialog.add(&page);
        dialog.present(Some(&window.window));
        Some(dialog)
    }

    fn switch_row(self: &Rc<Self>, key: &'static str, title: &str, subtitle: &str, background: bool) -> adw::SwitchRow {
        let row =
            adw::SwitchRow::builder().title(title).subtitle(subtitle).active(self.db.get_setting(key) == "1").build();
        let app = self.clone();
        row.connect_active_notify(move |row| {
            app.db.set_setting(key, if row.is_active() { "1" } else { "0" });
            if background {
                app.apply_background_setting();
            }
        });
        row
    }

    /// Os widgets guardam o texto já traduzido: recriar a janela aplica o idioma sem reiniciar.
    pub fn change_language(self: &Rc<Self>, code: &str, dialog: &impl IsA<adw::Dialog>) {
        self.db.set_setting("language", code);
        i18n::set_language(code);
        dialog.close();
        let Some(old) = self.window() else { return };
        let window = Window::new(self, old.source());
        *self.window.borrow_mut() = Some(window.clone());
        self.apply_background_setting();
        window.set_syncing(self.sync.busy());
        window.present();
        old.window.destroy();
        let app = self.clone();
        glib::idle_add_local_once(move || {
            app.show_preferences();
        });
    }
}

fn opml_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("OPML"));
    filter.add_pattern("*.opml");
    filter.add_pattern("*.xml");
    filter
}
